//! dsh(DeepSeek Harness)子进程管理与启动流水线。
//!
//! 生命周期:checking(环境检查)→ installing(npm 安装)→ starting(启动 dsh web)
//! → ready(服务就绪,窗口导航到 dsh Web UI)。
//! 状态迁移经 `boot-state` 事件推给前端,日志行经 `boot-log` 事件转发。
//!
//! 调研要点(见 docs/research):
//! - `dsh web` 默认 127.0.0.1:3080,支持 `--port 0`(OS 自动分配,避免端口冲突)
//! - 就绪信号 = stdout 打印 `dsh web: http://127.0.0.1:<port>`(源码注释明确是 readiness signal)
//! - 无 install 子命令,"安装" = npm 下载 @deepseek-ai/dsh
//! - dsh 不会自动打开系统浏览器(源码确认)

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// dsh 锁定版本。npm 包为 developer preview(官方警告有破坏性变更),锁版本防止意外升级。
pub const DSH_VERSION: &str = "0.1.0-rc.6";
/// dsh 要求的 Node 版本(仓库根 package.json engines):^22.19 || >=24
const NODE_REQ: &str = "Node.js ^22.19 或 >=24";
/// dsh 源码注释明确:"This URL line is a readiness signal" —— stdout 打印即服务就绪
const READY_PREFIX: &str = "dsh web: http://";
/// 启动就绪等待上限。
/// 实测:首次运行 dsh 需初始化 ~/.dsh profile + 加载 100+ 插件,约 65s 才打印就绪行。
/// 留足余量用 180s;二次启动(profile 已存在)通常 < 10s。
const START_TIMEOUT: Duration = Duration::from_secs(180);
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
        }
    }

    /// 自管安装目录:%APPDATA%\app.deepseek-desktop\dsh-runtime
    fn runtime_dir(&self) -> Result<PathBuf, String> {
        self.app
            .path()
            .app_data_dir()
            .map(|d| d.join("dsh-runtime"))
            .map_err(|e| format!("无法定位应用数据目录: {e}"))
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
        let s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        BootStateSnapshot {
            phase: s.phase,
            error: s.error.clone(),
            logs: self.recent_logs(40),
        }
    }

    fn recent_logs(&self, n: usize) -> Vec<LogLine> {
        self.state
            .lock()
            .map(|s| s.logs.iter().rev().take(n).cloned().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .rev()
            .collect()
    }

    /// 阶段迁移:更新状态并推送 `boot-state` 事件(仅阶段,不含日志)
    fn set_phase(&self, phase: Phase, error: Option<String>) {
        if let Ok(mut s) = self.state.lock() {
            s.phase = phase;
            s.error = error.clone();
        }
        let _ = self.app.emit("boot-state", BootStateView { phase, error });
    }

    fn set_error(&self, msg: String) {
        self.set_phase(Phase::Error, Some(msg));
    }

    /// 追加日志行(剥 ANSI、去尾空行),仅入环形缓冲供异常时附上下文。
    /// 不推流:正常流程前端只显示阶段 + 进度,避免安装期高频事件压垮渲染进程。
    fn push_log(&self, stream: &str, line: String) {
        let line = strip_ansi(&line);
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return;
        }
        if let Ok(mut s) = self.state.lock() {
            s.logs.push_back(LogLine { stream: stream.into(), line: trimmed.to_string() });
            while s.logs.len() > LOG_CAP {
                s.logs.pop_front();
            }
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

/// 检查 node 是否可用且满足版本要求
fn check_node() -> Result<String, String> {
    let out = Command::new("node")
        .arg("--version")
        .output()
        .map_err(|_| "未检测到 Node.js,请先安装 Node.js 22 LTS 或 24 后重试".to_string())?;
    if !out.status.success() {
        return Err("未检测到 Node.js,请先安装 Node.js 22 LTS 或 24 后重试".into());
    }
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let v = ver.trim_start_matches('v');
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
    Ok(ver)
}

/// 确保运行时目录存在并写入锁版本的 package.json
fn ensure_runtime(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("无法创建运行目录 {}: {e}", dir.display()))?;
    let pkg = dir.join("package.json");
    if !pkg.exists() {
        let content = format!(
            "{{\n  \"name\": \"deepseek-desktop-dsh-runtime\",\n  \"private\": true,\n  \"version\": \"0.1.0\",\n  \"dependencies\": {{\n    \"@deepseek-ai/dsh\": \"{DSH_VERSION}\"\n  }}\n}}\n"
        );
        fs::write(&pkg, content).map_err(|e| format!("无法写入 {}: {e}", pkg.display()))?;
    }
    Ok(())
}

/// 已安装的 dsh 版本(读 node_modules 里的 package.json)
fn installed_version(dir: &Path) -> Option<String> {
    let p = dir.join("node_modules/@deepseek-ai/dsh/package.json");
    let content = fs::read_to_string(p).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("version").and_then(|v| v.as_str()).map(String::from)
}

/// npm 安装 dsh(锁定版本),stdout/stderr 逐行转发为日志事件
fn npm_install(manager: &DshManager, dir: &Path) -> Result<(), String> {
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let mut cmd = Command::new(npm);
    cmd.current_dir(dir)
        .args(["install", "--no-audit", "--no-fund", "--no-progress"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 npm 安装: {e}"))?;

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

    let status = child.wait().map_err(|e| format!("npm 安装进程异常: {e}"))?;
    if !status.success() {
        return Err(format!("dsh 安装失败(退出码 {})", status.code().unwrap_or(-1)));
    }
    Ok(())
}

/// 启动 `node <bin.js> web --port 0`,返回 stdout/stderr 合流后的行接收端
fn spawn_dsh(manager: &DshManager, bin: &Path) -> Result<Receiver<(String, String)>, String> {
    let mut cmd = Command::new("node");
    cmd.arg(bin)
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

/// 从就绪行提取端口:形如 `dsh web: http://127.0.0.1:PORT`
fn parse_ready_line(line: &str) -> Option<u16> {
    let idx = line.find(READY_PREFIX)?;
    let rest = &line[idx + READY_PREFIX.len()..];
    let (_, port_part) = rest.rsplit_once(':')?;
    port_part.trim().parse::<u16>().ok()
}

/// 兜底就绪确认:轮询端口连通(最多 30 × 500ms),零 HTTP 依赖
fn tcp_wait(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    for _ in 0..30 {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(500));
    }
    false
}

/// 等待就绪信号行,返回端口与消费中的接收端(供 reaper 继续排空)。
/// 超时/进程退出/输出流关闭都视为失败。
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
                let exited = match manager.child.lock() {
                    Ok(mut guard) => match guard.as_mut() {
                        Some(c) => c.try_wait().ok().flatten().is_some(),
                        None => true,
                    },
                    Err(_) => true,
                };
                if exited {
                    return Err("dsh 启动失败(进程提前退出)".into());
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "dsh 启动超时({}s 内未收到就绪信号)",
                        START_TIMEOUT.as_secs()
                    ));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err("dsh 启动失败(输出流已关闭)".into());
            }
        }
    }
}

/// 杀掉 dsh 子进程(Windows 上是 TerminateProcess),幂等。
pub fn kill_child(manager: &DshManager) {
    if let Some(mut child) = manager.take_child() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

// ── boot 流水线(运行在工作线程上)─────────────────────────────────

fn boot_pipeline(manager: &DshManager) {
    // 1. 环境检查
    eprintln!("[dsh] boot: checking node…");
    manager.set_phase(Phase::Checking, None);
    if let Err(e) = check_node() {
        manager.set_error(e);
        return;
    }

    // 2. 运行时目录
    let runtime = match manager.runtime_dir() {
        Ok(r) => r,
        Err(e) => {
            manager.set_error(e);
            return;
        }
    };
    if let Err(e) = ensure_runtime(&runtime) {
        manager.set_error(e);
        return;
    }

    // 3. 安装(快速路径:已装且版本匹配则跳过)
    if installed_version(&runtime).as_deref() != Some(DSH_VERSION) {
        eprintln!("[dsh] boot: installing ({} → {})", runtime.display(), DSH_VERSION);
        manager.set_phase(Phase::Installing, None);
        if let Err(e) = npm_install(manager, &runtime) {
            manager.set_error(e);
            return;
        }
        if installed_version(&runtime).as_deref() != Some(DSH_VERSION) {
            manager.set_error("dsh 安装后版本校验失败".into());
            return;
        }
    } else {
        eprintln!("[dsh] boot: already installed, skip install");
    }

    // 4. 启动 dsh web
    eprintln!("[dsh] boot: starting dsh web…");
    manager.set_phase(Phase::Starting, None);
    let bin = runtime.join("node_modules/@deepseek-ai/dsh/lib/bin.js");
    if !bin.exists() {
        manager.set_error(format!("dsh 可执行文件缺失: {}", bin.display()));
        return;
    }
    let rx = match spawn_dsh(manager, &bin) {
        Ok(rx) => rx,
        Err(e) => {
            manager.set_error(e);
            return;
        }
    };

    // 5. 等待就绪信号
    let (port, rx) = match wait_ready(manager, rx) {
        Ok(p) => p,
        Err(e) => {
            kill_child(manager);
            manager.set_error(e);
            return;
        }
    };
    eprintln!("[dsh] boot: ready on port {port}");

    // 6. 就绪:导航窗口到 dsh Web UI,窗口自此变纯 dsh 页面(只做显示,不干扰功能)
    manager.set_phase(Phase::Ready, None);
    let url = format!("http://127.0.0.1:{port}");
    if let Some(win) = manager.app.get_webview_window("main") {
        if let Ok(u) = tauri::Url::parse(&url) {
            eprintln!("[dsh] boot: navigate → {url}");
            let _ = win.navigate(u);
        }
    }

    // 7. reaper 线程:持续排空输出流(防 64KB 管道阻塞挂死),进程退出后收割
    spawn_reaper(manager.clone(), rx);
}

/// 收割线程:排空 channel 直到读线程结束(进程退出或被杀),再 wait 回收。
/// 若 dsh 意外退出(非主动退出流程)则弹原生提示。
fn spawn_reaper(manager: DshManager, rx: Receiver<(String, String)>) {
    thread::spawn(move || {
        // 持续排空(丢弃);读线程随子进程退出而结束,tx 全部 drop 后 recv 返回 Err
        while rx.recv().is_ok() {}

        let child = manager.take_child();
        if let Some(mut c) = child {
            let status = c.wait();
            let ok = status.map(|s| s.success()).unwrap_or(false);
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
    let m = manager.clone();
    thread::spawn(move || {
        // catch_unwind:流水线线程 panic 会导致 phase 永远停在 Checking(前端永久 loading),
        // 捕获后转为可见错误,便于诊断
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| boot_pipeline(&m)));
        BOOTING.store(false, Ordering::SeqCst);
        if let Err(panic) = result {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "boot 流水线内部错误".into());
            eprintln!("[dsh] boot pipeline panic: {msg}");
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
