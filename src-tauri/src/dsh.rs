//! dsh(DeepSeek Harness)子进程管理与启动流水线。
//!
//! 生命周期:checking(环境检查)→ installing(npm 安装)→ starting(启动 dsh web)
//! → ready(服务就绪,窗口导航到 dsh Web UI)。
//! 状态迁移经 `boot-state` 事件推给前端;日志不推流,只入环形缓冲(异常时附在错误页)。
//!
//! 生产日志:本模块不直接 eprintln,统一走 `log` crate 宏(logging::init 落盘到
//! `<app_data_dir>/logs/app.log`,panic 经 hook 同落盘)。
//!
//! 安装策略(用户拍板):dsh 装到 **npm 全局**——「有则用,无则装」:
//! - 全局已有 dsh(任意可用版本)直接用,不重装、不比较版本、不强制升级
//! - 完全没有 → `npm install -g @deepseek-ai/dsh@latest`(bundle 离线缓存兜底待 #6 产物接入)
//! - 升级是独立的用户确认流程,boot 不阻塞在版本上
//!
//! 调研要点(见 docs/research):
//! - `dsh web` 默认 127.0.0.1:3080,支持 `--port 0`(OS 自动分配,避免端口冲突)
//! - 就绪信号 = stdout 打印 `dsh web: http://127.0.0.1:<port>`(源码注释明确是 readiness signal)
//! - 全局 bin:`dsh.cmd` 是 shim,真身 `{prefix}/node_modules/@deepseek-ai/dsh/lib/bin.js`,spawn node 绝对路径
//! - prefix 必须运行时解析(`npm root -g`),nvm 等环境不是 %APPDATA%\npm
//! - dsh 不会自动打开系统浏览器(源码确认)

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// dsh 要求的 Node 版本(仓库根 package.json engines):^22.19 || >=24
const NODE_REQ: &str = "Node.js ^22.19 或 >=24";
/// dsh 源码注释明确:"This URL line is a readiness signal" —— stdout 打印即服务就绪
const READY_PREFIX: &str = "dsh web: http://";
/// 启动就绪等待上限。
/// 实测:首次运行 dsh 需初始化 ~/.dsh profile + 加载 100+ 插件,约 65s 才打印就绪行。
/// 留足余量用 180s;二次启动(profile 已存在)通常 < 10s。
const START_TIMEOUT: Duration = Duration::from_secs(180);
/// `node --version` 检查超时。同步 output() 无上限:node 被 shim/杀软/网络盘挂起时
/// checking 会永久卡住,必须设上限(超时后杀进程并报可读错误)。
const CHECK_NODE_TIMEOUT: Duration = Duration::from_secs(10);
/// npm 安装超时。冷缓存首次安装可能要几分钟,给足 10 分钟;超时视为失败并报可读错误。
const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 日志环形缓冲容量(仅供异常时附上下文,不推流)
const LOG_CAP: usize = 200;

// ── 全局守卫(跨线程)────────────────────────────────────────────────

static QUITTING: AtomicBool = AtomicBool::new(false);
static DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);
/// 流水线运行中标志:防 StrictMode 双 invoke / setup 与前端同时触发导致双流水线竞态
static BOOTING: AtomicBool = AtomicBool::new(false);

/// 应用已进入程序化退出流程(放行 CloseRequested,不再弹对话框)
pub fn set_quitting() {
    QUITTING.store(true, Ordering::SeqCst);
}
pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}
/// 关闭对话框防双触发(CloseRequested 在 webview 与 window 层各触发一次)
pub fn try_show_dialog() -> bool {
    DIALOG_SHOWN
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}
pub fn reset_dialog_flag() {
    DIALOG_SHOWN.store(false, Ordering::SeqCst);
}

// ── 状态与视图 ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Checking,
    Installing,
    Starting,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// "stdout" | "stderr"
    pub stream: String,
    pub line: String,
}

/// boot-state 事件/命令返回:只含阶段,不含日志(减重,高频推送不影响渲染)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootStateView {
    pub phase: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// get_boot_state 快照:含最近日志(仅挂载时拉取一次)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootStateSnapshot {
    pub phase: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub logs: Vec<LogLine>,
}

struct BootState {
    phase: Phase,
    error: Option<String>,
    logs: VecDeque<LogLine>,
}

/// dsh 生命周期管理器。Clone 共享内部状态(boot 线程与 reaper 线程各持一份)。
#[derive(Clone)]
pub struct DshManager {
    app: AppHandle,
    state: Arc<Mutex<BootState>>,
    child: Arc<Mutex<Option<Child>>>,
    /// 安装中 npm 子进程 pid(退出收敛时一并杀掉;npm 会再拉起 node 子进程,按树杀)
    install_pid: Arc<Mutex<Option<u32>>>,
}

impl DshManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            state: Arc::new(Mutex::new(BootState {
                phase: Phase::Idle,
                error: None,
                logs: VecDeque::new(),
            })),
            child: Arc::new(Mutex::new(None)),
            install_pid: Arc::new(Mutex::new(None)),
        }
    }

    fn phase(&self) -> Phase {
        self.state
            .lock()
            .map(|s| s.phase)
            .unwrap_or(Phase::Error)
    }

    fn view(&self) -> BootStateView {
        let s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        BootStateView { phase: s.phase, error: s.error.clone() }
    }

    fn snapshot(&self) -> BootStateSnapshot {
        // 注意:不能在持锁时再调需要同一把锁的方法(会死锁),日志先在此处取出
        let s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let logs = s
            .logs
            .iter()
            .rev()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        BootStateSnapshot { phase: s.phase, error: s.error.clone(), logs }
    }

    /// 阶段迁移:更新状态并推送 `boot-state` 事件(仅阶段,不含日志)
    fn set_phase(&self, phase: Phase, error: Option<String>) {
        if let Ok(mut s) = self.state.lock() {
            // 新一次 boot 的语义边界:进入 Checking 时清空上一轮的日志缓冲
            if phase == Phase::Checking {
                s.logs.clear();
            }
            s.phase = phase;
            s.error = error.clone();
        }
        match &error {
            Some(e) => log::error!("[dsh] phase → {phase:?}: {e}"),
            None => log::info!("[dsh] phase → {phase:?}"),
        }
        let _ = self.app.emit("boot-state", BootStateView { phase, error });
    }

    fn set_error(&self, msg: String) {
        self.set_phase(Phase::Error, Some(msg));
    }

    /// 追加日志行(剥 ANSI、去尾空行),仅入环形缓冲供异常时附上下文。
    /// 不推流:正常流程前端只显示阶段 + 进度,避免安装期高频事件压垮渲染进程。
    /// boot 阶段(安装/启动)的日志同时落盘到日志文件;ready 后 dsh 运行时输出
    /// 不写日志文件(避免每请求日志撑爆文件)。
    fn push_log(&self, stream: &str, line: String) {
        let line = strip_ansi(&line);
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return;
        }
        let to_file = self
            .state
            .lock()
            .map(|mut s| {
                s.logs.push_back(LogLine { stream: stream.into(), line: trimmed.to_string() });
                while s.logs.len() > LOG_CAP {
                    s.logs.pop_front();
                }
                s.phase != Phase::Ready
            })
            .unwrap_or(false);
        if to_file {
            log::info!("[dsh:{stream}] {trimmed}");
        }
    }

    fn set_child(&self, child: Child) {
        if let Ok(mut g) = self.child.lock() {
            *g = Some(child);
        }
    }

    fn take_child(&self) -> Option<Child> {
        self.child.lock().ok().and_then(|mut g| g.take())
    }

    fn set_install_pid(&self, pid: u32) {
        if let Ok(mut g) = self.install_pid.lock() {
            *g = Some(pid);
        }
    }

    fn clear_install_pid(&self) {
        if let Ok(mut g) = self.install_pid.lock() {
            *g = None;
        }
    }

    fn take_install_pid(&self) -> Option<u32> {
        self.install_pid.lock().ok().and_then(|mut g| g.take())
    }
}

// ── 子进程工具 ─────────────────────────────────────────────────────

/// 带超时的子进程收割:轮询 try_wait 直到退出或超时。
/// **返回 Timeout 时子进程仍在运行,由调用方负责终止**(按进程树杀,见 kill_pid_tree)。
#[derive(Debug, PartialEq, Eq)]
enum ChildWaitError {
    Timeout(Duration),
    Io(String),
}

fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> Result<Output, ChildWaitError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // 进程已退出:收集 stdout/stderr(调用方未 take 的部分)
                use std::io::Read;
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut so) = child.stdout.take() {
                    let _ = so.read_to_end(&mut stdout);
                }
                if let Some(mut se) = child.stderr.take() {
                    let _ = se.read_to_end(&mut stderr);
                }
                return Ok(Output { status, stdout, stderr });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err(ChildWaitError::Timeout(timeout));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(ChildWaitError::Io(e.to_string())),
        }
    }
}

/// Windows 上隐藏子进程控制台窗口:GUI 应用(无控制台)直接 spawn node/npm 会闪
/// 一个 console 窗口。CREATE_NO_WINDOW = 0x08000000。
#[cfg(windows)]
fn no_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}

#[cfg(not(windows))]
fn no_window(cmd: &mut Command) -> &mut Command {
    cmd
}

/// 按进程树杀:Windows 用 taskkill /T /F(CreateProcess 只杀直接子进程,node 拉起的
/// 孙进程会成孤儿);Unix 用 kill 命令。幂等:进程已退出时静默失败。
fn kill_pid_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut binding = Command::new("taskkill");
        let _ = no_window(&mut binding)
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/// 杀掉 dsh 子进程(进程树)与安装中的 npm 进程,幂等。
pub fn kill_child(manager: &DshManager) {
    if let Some(mut child) = manager.take_child() {
        log::info!("[dsh] kill dsh 子进程 pid={}", child.id());
        kill_pid_tree(child.id());
        let _ = child.wait();
    }
    if let Some(pid) = manager.take_install_pid() {
        log::info!("[dsh] kill npm 安装进程 pid={pid}");
        kill_pid_tree(pid);
    }
}

/// 当前 dsh 子进程的退出码。None = 仍在运行/句柄丢失。
/// try_wait 出错(句柄异常/已被他处收割)按已退出处理:立即报「进程提前退出」进错误页,
/// 不干等 180s 超时(半残安装 + 秒崩场景 5s 内到错误页)。
fn child_exit_code(manager: &DshManager) -> Option<i32> {
    let mut guard = manager.child.lock().ok()?;
    let child = guard.as_mut()?;
    match child.try_wait() {
        Ok(Some(s)) => Some(s.code().unwrap_or(-1)),
        Ok(None) => None,
        Err(_) => Some(-1),
    }
}

// ── 工具函数 ───────────────────────────────────────────────────────

/// 剥离 ANSI 色码序列(ESC [ ... 字母)。就绪行与日志行都可能带颜色。
fn strip_ansi(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
            // 跳过 CSI 序列:参数直到终止字符(@~A-Za-z)
            let mut j = i + 2;
            while j < chars.len() && !chars[j].is_ascii_alphabetic() && chars[j] != '~' {
                j += 1;
            }
            i = j.saturating_add(1).min(chars.len());
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// 校验 node 版本是否满足 dsh 要求(^22.19 || >=24)。纯函数,可测试。
fn check_node_version(ver: &str) -> Result<(), String> {
    let v = ver.trim().trim_start_matches('v');
    let mut parts = v.split('.');
    let major: u32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("无法解析 Node 版本: {ver}"))?;
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ok = (major == 22 && minor >= 19) || major >= 24;
    if !ok {
        return Err(format!("Node 版本不满足要求:当前 {ver},需要 {NODE_REQ}"));
    }
    Ok(())
}

/// 检查 node 是否可用且满足版本要求。`node --version` 带超时:
/// node 被 shim/杀软/网络盘挂起时同步 output() 会永久阻塞,超时后杀进程并报可读错误。
fn check_node() -> Result<String, String> {
    let mut binding = Command::new("node");
    let mut child = no_window(&mut binding)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "未检测到 Node.js,请先安装 Node.js 22 LTS 或 24 后重试".to_string())?;
    let out = match wait_with_timeout(&mut child, CHECK_NODE_TIMEOUT) {
        Ok(out) => out,
        Err(ChildWaitError::Timeout(_)) => {
            kill_pid_tree(child.id());
            let _ = child.wait();
            return Err(format!(
                "Node.js 检查超时({}s 无响应):`node --version` 未返回。可能是 node 安装异常或被杀软拦截,请检查后重试",
                CHECK_NODE_TIMEOUT.as_secs()
            ));
        }
        Err(ChildWaitError::Io(e)) => return Err(format!("Node.js 检查失败: {e}")),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if stderr.is_empty() { String::new() } else { format!("({stderr})") };
        return Err(format!(
            "Node.js 检查失败:`node --version` 退出码 {}{detail}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    check_node_version(&ver)?;
    Ok(ver)
}

/// 全局 node_modules 路径,运行时动态解析(`npm root -g`)。
/// 不可写死 %APPDATA%\npm:nvm 等环境 prefix 不同(本机实测 nvm 下为 E:\Nvm\nodejs)。
/// 带超时:`npm root -g` 也会拉起 node,npm/node 被挂起时不得让 boot 卡死在检查阶段。
const NPM_ROOT_TIMEOUT: Duration = Duration::from_secs(10);

fn global_node_modules() -> Result<PathBuf, String> {
    let mut cmd = Command::new(if cfg!(windows) { "cmd.exe" } else { "npm" });
    if cfg!(windows) {
        cmd.args(["/c", "npm.cmd"]);
    }
    no_window(&mut cmd)
        .args(["root", "-g"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|_| "无法执行 npm,请确认 npm 可用".to_string())?;
    let out = match wait_with_timeout(&mut child, NPM_ROOT_TIMEOUT) {
        Ok(out) => out,
        Err(ChildWaitError::Timeout(_)) => {
            kill_pid_tree(child.id());
            let _ = child.wait();
            return Err(format!(
                "npm root -g 超时({}s 无响应),无法定位全局 dsh。请检查 npm 是否正常",
                NPM_ROOT_TIMEOUT.as_secs()
            ));
        }
        Err(ChildWaitError::Io(e)) => return Err(format!("npm root -g 失败: {e}")),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if stderr.is_empty() { String::new() } else { format!("({stderr})") };
        return Err(format!(
            "获取 npm 全局目录失败(退出码 {}){detail}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return Err("npm 全局目录为空".into());
    }
    Ok(PathBuf::from(path))
}

/// 全局 dsh bin.js 路径 + 完整性校验(纯函数,可测试)。
/// 校验 bin.js 存在而非仅版本号——半残安装(版本号在、文件缺)不得视为「已安装」,
/// 否则坏掉的安装会被「有则用」跳过,永远修不好(2026-08-14 实测事故)。
fn dsh_bin_path(global_node_modules: &Path) -> Option<PathBuf> {
    let bin = global_node_modules.join("@deepseek-ai/dsh/lib/bin.js");
    bin.exists().then_some(bin)
}

fn global_dsh_bin() -> Option<PathBuf> {
    dsh_bin_path(&global_node_modules().ok()?)
}

/// npm 全局安装 dsh(跟随 latest),stdout/stderr 逐行转发为日志事件。
/// Windows 上 CreateProcess 不能直接执行 .cmd/.bat(npm 是 .cmd shim),须经 cmd.exe /c 包装。
/// 带 NPM_INSTALL_TIMEOUT 超时;超时按进程树杀并报可读错误。
fn npm_install_global(manager: &DshManager) -> Result<(), String> {
    let mut cmd = Command::new(if cfg!(windows) { "cmd.exe" } else { "npm" });
    if cfg!(windows) {
        cmd.args(["/c", "npm.cmd"]);
    }
    no_window(&mut cmd)
        .args(["install", "-g", "@deepseek-ai/dsh@latest", "--no-audit", "--no-fund", "--no-progress"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 npm 安装: {e}"))?;
    // 登记安装中进程,退出收敛时一并杀(quit_app/托盘退出)
    manager.set_install_pid(child.id());

    let stdout = child.stdout.take().expect("piped stdout");
    let m = manager.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            m.push_log("stdout", line);
        }
    });
    let stderr = child.stderr.take().expect("piped stderr");
    let m = manager.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            m.push_log("stderr", line);
        }
    });

    let result = wait_with_timeout(&mut child, NPM_INSTALL_TIMEOUT);
    manager.clear_install_pid();
    match result {
        Ok(out) => {
            if out.status.success() {
                Ok(())
            } else {
                Err(format!("dsh 安装失败(退出码 {})", out.status.code().unwrap_or(-1)))
            }
        }
        Err(ChildWaitError::Timeout(_)) => {
            kill_pid_tree(child.id());
            let _ = child.wait();
            Err(format!(
                "dsh 安装超时({}s 内未完成)。请检查网络后点击重试",
                NPM_INSTALL_TIMEOUT.as_secs()
            ))
        }
        Err(ChildWaitError::Io(e)) => Err(format!("npm 安装进程异常: {e}")),
    }
}

/// 启动 `node <bin.js> web --port 0`,返回 stdout/stderr 合流后的行接收端
fn spawn_dsh(manager: &DshManager, bin: &Path) -> Result<Receiver<(String, String)>, String> {
    let mut binding = Command::new("node");
    let cmd = no_window(&mut binding)
        .arg(bin)
        .args(["web", "--port", "0"])
        .env("DSH_TELEMETRY_DISABLED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 dsh: {e}"))?;

    let (tx, rx) = mpsc::channel::<(String, String)>();
    let stdout = child.stdout.take().expect("piped stdout");
    let tx2 = tx.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx2.send(("stdout".into(), line)).is_err() {
                break;
            }
        }
    });
    let stderr = child.stderr.take().expect("piped stderr");
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(("stderr".into(), line)).is_err() {
                break;
            }
        }
    });

    manager.set_child(child);
    Ok(rx)
}

/// 从就绪行提取端口:形如 `dsh web: http://127.0.0.1:PORT`。
/// 容错:先剥 ANSI(就绪行可能带颜色),再取最后一个冒号后的数字前缀
/// (容忍尾随路径/查询串/空白,如 `...:3080/`、`...:3080?x=1`)。
fn parse_ready_line(line: &str) -> Option<u16> {
    let clean = strip_ansi(line);
    let idx = clean.find(READY_PREFIX)?;
    let rest = &clean[idx + READY_PREFIX.len()..];
    let port_part = rest.rsplit(':').next()?.trim();
    let digits: String = port_part.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u16>().ok()
}

/// 兜底就绪确认:轮询端口连通(attempts × connect_timeout),零 HTTP 依赖。
/// 生产调用 tcp_wait(30 次 × 500ms);attempts/timeout 可注入供测试用短参数。
fn tcp_wait_attempts(port: u16, attempts: u32, timeout: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    for _ in 0..attempts {
        if TcpStream::connect_timeout(&addr, timeout).is_ok() {
            return true;
        }
        thread::sleep(timeout);
    }
    false
}

fn tcp_wait(port: u16) -> bool {
    tcp_wait_attempts(port, 30, Duration::from_millis(500))
}

/// 等待就绪信号行,返回端口与消费中的接收端(供 reaper 继续排空)。
/// 超时/进程退出/输出流关闭都视为失败,错误信息带退出码便于诊断。
fn wait_ready(
    manager: &DshManager,
    rx: Receiver<(String, String)>,
) -> Result<(u16, Receiver<(String, String)>), String> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok((stream, line)) => {
                manager.push_log(&stream, line.clone());
                if let Some(port) = parse_ready_line(&line) {
                    if tcp_wait(port) {
                        return Ok((port, rx));
                    }
                    return Err(format!("dsh 已打印就绪地址但端口 {port} 未监听"));
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // 子进程是否已退出(锁作用域内完成判断,避免守卫借用逃逸)
                if let Some(code) = child_exit_code(manager) {
                    return Err(format!("dsh 启动失败(进程提前退出,退出码 {code})"));
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "dsh 启动超时({}s 内未收到就绪信号)",
                        START_TIMEOUT.as_secs()
                    ));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // 输出流关闭 = 读线程结束 = 子进程已退出(或句柄被继承导致异常断流)
                let code = child_exit_code(manager)
                    .map(|c| format!(",退出码 {c}"))
                    .unwrap_or_default();
                return Err(format!("dsh 启动失败(进程提前退出{code})"));
            }
        }
    }
}

// ── boot 流水线(运行在工作线程上)─────────────────────────────────

fn boot_pipeline(manager: &DshManager) {
    // 1. 环境检查
    log::info!("[dsh] boot: checking node…");
    manager.set_phase(Phase::Checking, None);
    if let Err(e) = check_node() {
        manager.set_error(e);
        return;
    }

    // 2. 全局 dsh 检测(完整性校验:bin.js 存在,不只版本号)
    //    「有则用」:全局已有 dsh(任意可用版本)直接用,不重装不强制升级
    let bin = match global_dsh_bin() {
        Some(b) => b,
        None => {
            log::info!("[dsh] boot: global dsh not found, installing…");
            manager.set_phase(Phase::Installing, None);
            if let Err(e) = npm_install_global(manager) {
                manager.set_error(e);
                return;
            }
            // 安装后完整性复检:装完仍然不可用视为失败
            match global_dsh_bin() {
                Some(b) => b,
                None => {
                    manager.set_error("dsh 安装后校验失败(可执行文件缺失)".into());
                    return;
                }
            }
        }
    };

    // 3. 启动 dsh web
    log::info!("[dsh] boot: starting dsh web…");
    manager.set_phase(Phase::Starting, None);
    let rx = match spawn_dsh(manager, &bin) {
        Ok(rx) => rx,
        Err(e) => {
            manager.set_error(e);
            return;
        }
    };

    // 4. 等待就绪信号
    let (port, rx) = match wait_ready(manager, rx) {
        Ok(p) => p,
        Err(e) => {
            kill_child(manager);
            manager.set_error(e);
            return;
        }
    };
    log::info!("[dsh] boot: ready on port {port}");

    // 6. 就绪:导航窗口到 dsh Web UI,窗口自此变纯 dsh 页面(只做显示,不干扰功能)
    manager.set_phase(Phase::Ready, None);
    let url = format!("http://127.0.0.1:{port}");
    if let Some(win) = manager.app.get_webview_window("main") {
        if let Ok(u) = tauri::Url::parse(&url) {
            log::info!("[dsh] boot: navigate → {url}");
            if let Err(e) = win.navigate(u) {
                // 导航失败:dsh 仍在运行,先杀子进程再进错误态(否则重试会再起一个 dsh)
                log::error!("[dsh] navigate 失败: {e}");
                kill_child(manager);
                manager.set_error("无法打开 dsh 页面,请重试".into());
                return;
            }
        }
    }

    // 7. reaper 线程:持续排空输出流(防 64KB 管道阻塞挂死),进程退出后收割
    spawn_reaper(manager.clone(), rx);
}

/// 收割线程:排空 channel 直到读线程结束(进程退出或被杀),再 wait 回收。
/// 若 dsh 意外退出(非主动退出流程)则弹原生提示。
///
/// 竞态防护:只有 phase 仍为 Ready 时才取子进程句柄。若排空期间用户已重试
/// (phase 进入 Checking),新 boot 的 child 已写入 manager——此时取走会令
/// 新 boot 的 wait_ready 误判"进程提前退出",故直接放弃收割。
fn spawn_reaper(manager: DshManager, rx: Receiver<(String, String)>) {
    thread::spawn(move || {
        // 持续排空(丢弃);读线程随子进程退出而结束,tx 全部 drop 后 recv 返回 Err
        while rx.recv().is_ok() {}

        if manager.phase() != Phase::Ready {
            return;
        }
        let child = manager.take_child();
        if let Some(mut c) = child {
            let status = c.wait();
            let ok = status.map(|s| s.success()).unwrap_or(false);
            log::info!("[dsh] reaper: dsh 子进程退出, ok={ok}, quitting={}", is_quitting());
            if !ok && !is_quitting() {
                let _ = manager
                    .app
                    .dialog()
                    .message("dsh 进程意外退出,请重新启动应用")
                    .title("DeepSeek Desktop")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }
        }
    });
}

// ── Tauri commands ─────────────────────────────────────────────────

/// 启动(或重试)boot 流水线。phase 守卫:非 Idle/Error 时 no-op(防 StrictMode 双 invoke)。
/// async + Result:不占用 IPC/主线程(tauri 2 要求含引用输入的 async command 返回 Result)。
#[tauri::command]
pub async fn boot(state: tauri::State<'_, DshManager>) -> Result<BootStateView, String> {
    boot_start(state.inner());
    Ok(state.inner().view())
}

/// 在独立线程启动流水线;Idle(首启)/Error(重试)时才生效,其余阶段 no-op。
/// BOOTING 标志防并发双流水线(StrictMode 双 invoke + setup 主动启动)。
pub fn boot_start(manager: &DshManager) {
    let phase = manager.phase();
    if !matches!(phase, Phase::Idle | Phase::Error) {
        return;
    }
    if BOOTING.swap(true, Ordering::SeqCst) {
        return;
    }
    log::info!("[dsh] boot 流水线启动(phase={phase:?})");
    let m = manager.clone();
    thread::spawn(move || {
        // catch_unwind:流水线线程 panic 会导致 phase 永远停在 Checking(前端永久 loading),
        // 捕获后转为可见错误,便于诊断(panic 本身已由 logging hook 落盘)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| boot_pipeline(&m)));
        BOOTING.store(false, Ordering::SeqCst);
        if let Err(panic) = result {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "boot 流水线内部错误".into());
            log::error!("[dsh] boot 流水线 panic: {msg}");
            m.set_error(format!("内部错误:{msg}"));
        }
    });
}

/// 当前状态快照(前端挂载/重载时拉取,含最近日志)
#[tauri::command]
pub async fn get_boot_state(
    state: tauri::State<'_, DshManager>,
) -> Result<BootStateSnapshot, String> {
    Ok(state.inner().snapshot())
}

/// 退出应用:杀子进程 + exit(0)。程序化退出先置 QUITTING 标志放行 CloseRequested。
#[tauri::command]
pub async fn quit_app(
    app: tauri::AppHandle,
    state: tauri::State<'_, DshManager>,
) -> Result<(), String> {
    set_quitting();
    kill_child(state.inner());
    app.exit(0);
    Ok(())
}

// ── 测试 ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ready_line_extracts_port() {
        assert_eq!(parse_ready_line("dsh web: http://127.0.0.1:3080"), Some(3080));
        // 尾随斜杠 / 空白
        assert_eq!(parse_ready_line("dsh web: http://127.0.0.1:3080/"), Some(3080));
        assert_eq!(parse_ready_line("dsh web: http://127.0.0.1:3080   "), Some(3080));
        // 尾随路径/查询串:取数字前缀
        assert_eq!(parse_ready_line("dsh web: http://127.0.0.1:3080/?x=1"), Some(3080));
    }

    #[test]
    fn parse_ready_line_strips_ansi() {
        let line = "\u{1b}[32mdsh web: http://127.0.0.1:3080\u{1b}[0m";
        assert_eq!(parse_ready_line(line), Some(3080));
    }

    #[test]
    fn parse_ready_line_rejects_non_ready_lines() {
        assert_eq!(parse_ready_line("some other line"), None);
        assert_eq!(parse_ready_line("http://127.0.0.1:3080"), None); // 缺 "dsh web: " 前缀
        assert_eq!(parse_ready_line("dsh web: http://127.0.0.1:"), None);
        assert_eq!(parse_ready_line("dsh web: http://127.0.0.1:abc"), None);
        assert_eq!(parse_ready_line("dsh web: http://127.0.0.1:65536"), None); // 越界
    }

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        assert_eq!(strip_ansi("plain text"), "plain text");
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{1b}[2Kclear"), "clear");
        assert_eq!(strip_ansi("\u{1b}[38;5;196mcolor256\u{1b}[0m"), "color256");
        assert_eq!(strip_ansi("中文\u{1b}[1m加粗\u{1b}[0m保留"), "中文加粗保留");
    }

    #[test]
    fn check_node_version_accepts_required_ranges() {
        assert!(check_node_version("v22.22.2").is_ok());
        assert!(check_node_version("v22.19.0").is_ok());
        assert!(check_node_version("v24.0.0").is_ok());
        assert!(check_node_version("v25.1.0").is_ok());
        assert!(check_node_version("22.22.2").is_ok()); // 无 v 前缀
    }

    #[test]
    fn check_node_version_rejects_others() {
        assert!(check_node_version("v22.18.0").is_err()); // 22 系但 < 19
        assert!(check_node_version("v23.0.0").is_err()); // 23 不在 ^22.19 || >=24 内
        assert!(check_node_version("v20.10.0").is_err());
        assert!(check_node_version("not-a-version").is_err());
    }

    #[test]
    fn tcp_wait_detects_open_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(tcp_wait(port)); // 生产路径:30 × 500ms
    }

    #[test]
    fn tcp_wait_fails_on_closed_port() {
        // 生产路径 tcp_wait 对关闭端口要跑满 30 次 × 500ms ≈ 15s,测试注入短参数
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(!tcp_wait_attempts(port, 2, Duration::from_millis(50)));
    }

    #[cfg(windows)]
    #[test]
    fn wait_with_timeout_collects_output_of_quick_command() {
        let mut child = Command::new("cmd").args(["/c", "echo ok"]).stdout(Stdio::piped()).spawn().unwrap();
        let out = wait_with_timeout(&mut child, Duration::from_secs(5)).unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
    }

    #[cfg(windows)]
    #[test]
    fn wait_with_timeout_times_out_and_caller_kills() {
        // ping -n 3 ≈ 2s,超时 200ms;返回 Timeout 后按进程树杀
        let mut child = Command::new("cmd")
            .args(["/c", "ping -n 3 127.0.0.1 >nul"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let r = wait_with_timeout(&mut child, Duration::from_millis(200));
        assert!(matches!(r, Err(ChildWaitError::Timeout(_))), "期望 Timeout,实际 {r:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "超时检测过慢");
        kill_pid_tree(child.id());
        let _ = child.wait();
    }

    #[test]
    fn dsh_bin_path_requires_bin_js_not_just_version() {
        // 半残安装(版本号在、bin.js 缺)不得视为「已安装」——否则被「有则用」跳过,
        // 坏掉的安装永远修不好(2026-08-14 实测事故)
        let dir = std::env::temp_dir().join(format!(
            "dsh-boot-test-{}",
            std::process::id()
        ));
        let dsh_dir = dir.join("@deepseek-ai/dsh");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dsh_dir.join("lib")).unwrap();

        // 只有 package.json(无 bin.js)→ 视为未安装
        std::fs::write(dsh_dir.join("package.json"), r#"{"version":"0.1.0-rc.6"}"#).unwrap();
        assert_eq!(dsh_bin_path(&dir), None, "版本号存在但 bin.js 缺失必须判为未安装");

        // 补上 bin.js → 视为已安装
        std::fs::write(dsh_dir.join("lib/bin.js"), "// placeholder").unwrap();
        assert_eq!(
            dsh_bin_path(&dir),
            Some(dsh_dir.join("lib/bin.js")),
            "bin.js 存在即视为已安装(不比较版本)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn npm_cmd_spawns_via_cmd_exe() {
        // 生产路径:cmd.exe /c npm.cmd —— CreateProcess 不能直接执行 .cmd
        let out = Command::new("cmd.exe")
            .args(["/c", "npm.cmd", "--version"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(out.status.success(), "npm.cmd 经 cmd.exe 启动失败: {}", String::from_utf8_lossy(&out.stderr));
        assert!(!String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }
}
