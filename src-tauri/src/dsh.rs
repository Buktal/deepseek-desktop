//! dsh(DeepSeek Harness)子进程管理与启动流水线。
//!
//! 生命周期:checking(环境检查)→ installing(npm 安装)→ starting(启动 dsh web)
//! → ready(服务就绪,记录 URL 并推给壳页,dsh 在 iframe 内呈现)。
//! 状态迁移经 `boot-state` 事件推给前端(只 emit 到 `main` 窗口);日志不推流,
//! 只入环形缓冲(异常时附在错误页)。
//!
//! IPC 命令面(最小化,2 个):
//! - `boot`(触发/重试流水线 + 返回含日志与 dshUrl 的当前状态快照;挂载时一调两用)
//! - `quit_app`(程序化退出:杀子进程 + exit)
//!
//! dsh URL 单一事实来源:`record_dsh_url`(boot 就绪 / 升级链就绪 /「稍后/返回」
//! 重启 / 崩溃重试 4 个时点统一调用)在记录的同时推 `dsh-url` 事件给壳页,壳页
//! set iframe.src——整窗导航退役(ADR 0001 / #29,#36)。
//!
//! 安全语义(tauri 2.11.5 源码确认):
//! - dsh 以跨源 iframe 嵌入壳页(http://127.0.0.1:<port> 是 remote origin):
//!   ACL 按 capability(local-only)拒绝其调用任何命令/监听事件/使用窗口 API,
//!   与整窗模式一致(零 remote capability,#29);Tauri 的 app CSP 只注入资产
//!   协议提供的本地页面,dsh 页面的 CSP 归 dsh 服务器自身。
//!
//! 生产日志:本模块不直接 eprintln,统一走 `log` crate 宏(logging::init 落盘到
//! `<temp>/deepseek-desktop/logs/app.log`,panic 经 hook 同落盘)。
//!
//! 安装策略(用户拍板):dsh 装到 **npm 全局**——「有则用,无则装」:
//! - 全局已有 dsh(任意可用版本)直接用,不重装、不比较版本、不强制升级
//! - 完全没有 → `npm install -g @deepseek-ai/dsh@latest`;安装包内置离线缓存
//!   (约定 `<资源目录>/npm-cache`,cacache 目录,CI 发版时打包,见 #6)存在时
//!   加 `--prefer-offline --cache <目录>`:缓存命中秒级完成、缺失自动回退网络
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
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::DshError;
use crate::npm::{self, InstallStage};
use crate::proc::{kill_pid_tree, new_process_group, no_window, wait_with_timeout};

/// dsh 源码注释明确:"This URL line is a readiness signal" —— stdout 打印即服务就绪
const READY_PREFIX: &str = "dsh web: http://";
/// 启动就绪等待上限。
/// 实测:首次运行 dsh 需初始化 ~/.dsh profile + 加载 100+ 插件,约 65s 才打印就绪行。
/// 留足余量用 180s;二次启动(profile 已存在)通常 < 10s。
const START_TIMEOUT: Duration = Duration::from_secs(180);
/// 帧嵌入回归检查超时(本地 127.0.0.1 服务,3s 足够)。
const FRAME_CHECK_TIMEOUT: Duration = Duration::from_secs(3);
/// 日志环形缓冲容量(仅供异常时附上下文,不推流)
const LOG_CAP: usize = 200;

// ── 全局守卫(跨线程)────────────────────────────────────────────────

static QUITTING: AtomicBool = AtomicBool::new(false);
/// 流水线运行中标志:防 StrictMode 双 invoke / setup 与前端同时触发导致双流水线竞态
static BOOTING: AtomicBool = AtomicBool::new(false);
/// dsh 升级链主动杀旧 dsh 时的抑制标志(#3 §2):独立于 set_quitting——
/// 升级不退出应用、不要求放行 CloseRequested(关闭弹窗整个会话保持有效),
/// 只是不让 reaper 把「升级主动杀的旧 dsh」误判为「意外退出」弹窗。
/// 杀旧进程前置位、新 dsh 就绪(或升级失败收敛)后清除;清除时机安全:
/// 旧进程的 reaper 在进程退出(杀后即刻)时早已过判定点。
static UPGRADE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 应用已进入程序化退出流程(放行 CloseRequested,不再弹对话框)
pub fn set_quitting() {
    QUITTING.store(true, Ordering::SeqCst);
}
pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}
/// 置位/清除升级抑制标志(upgrade.rs 流水线调用;#3 §2,独立于 set_quitting)。
pub fn set_upgrade_active(v: bool) {
    UPGRADE_ACTIVE.store(v, Ordering::SeqCst);
}
pub fn upgrade_active() -> bool {
    UPGRADE_ACTIVE.load(Ordering::SeqCst)
}

// ── 状态与视图 ─────────────────────────────────────────────────────

/// Windows 任务栏图标进度状态(tauri set_progress_bar 语义的极简映射):
/// Clear = 隐藏进度,Indeterminate = 流动动画(checking/starting),
/// Percent(p) = 确定进度 0-100(installing)。
#[derive(Debug, Clone, Copy)]
enum TaskbarProgress {
    Clear,
    Indeterminate,
    Percent(u8),
}

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

/// dsh 服务运行状态(service_status 的组合判定结果,见 derive_status)。
/// 五个调用点(boot_start 守卫 / 升级确认守卫 / 升级卡恢复判断 / 手动检查
/// boot 就绪判定 / reaper 之外的状态消费)统一经此谓词,不再各自拼
/// phase×child×try_wait。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceStatus {
    /// 尚未启动(Idle)
    NotReady,
    /// 流水线运行中(Checking/Installing/Starting)
    Booting,
    /// 就绪且进程存活
    Running,
    /// 就绪但进程已退出或句柄缺失(意外退出,待重跑 boot)
    DeadAfterCrash,
    /// 错误态(Error)
    Error,
}

/// 由阶段与子进程存活两事实推导服务状态(纯函数,可穷举单测)。
/// 组合语义内化:只有 Ready 关注子进程死活——「Ready + 句柄空 = 意外退出
/// 待重跑」(reaper 已收割)、「try_wait 出错按已退出」(不干等 180s 超时);
/// 流水线在途一律 Booting(无论子进程是否已 spawn)。
fn derive_status(phase: Phase, child_alive: bool) -> ServiceStatus {
    match phase {
        Phase::Idle => ServiceStatus::NotReady,
        Phase::Error => ServiceStatus::Error,
        Phase::Ready => {
            if child_alive {
                ServiceStatus::Running
            } else {
                ServiceStatus::DeadAfterCrash
            }
        }
        // Checking / Installing / Starting
        _ => ServiceStatus::Booting,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// "stdout" | "stderr"
    pub stream: String,
    pub line: String,
}

/// boot-state 事件/命令返回:只含阶段与进度,不含日志(减重,高频推送不影响渲染)。
/// 进度字段仅 installing 阶段携带(progress/stage);node_version 仅 checking 阶段
/// 携带(检测结果可视化,见 set_node_version);elapsed_secs 从流水线启动起
/// 累计,checking/installing/starting 全程可显示耗时(见 BootScreen)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootStateView {
    pub phase: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DshError>,
    /// Node 检测结果(仅 checking 阶段携带,启动页显示「检测到 Node.js vX」)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_version: Option<String>,
    /// 安装模拟进度 0-100(None = 非安装阶段;100 只能由 npm 进程退出校准)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// 安装子阶段(前端拼 `boot.installing.stage.<stage>` 文案键)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<InstallStage>,
    /// 从流水线启动起的累计秒数(启动页耗时显示;进度 tick 附带)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
}

/// `boot` 命令返回的状态快照:含最近日志与当前进度(挂载/重试一调两用)。
/// dshUrl 在就绪过时携带(壳页挂载晚于 boot 完成时,快照是 dsh URL 的唯一
/// 来源——事件已错过,见 useBoot 的同步骨架与就绪缺 URL 兜底)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootStateSnapshot {
    pub phase: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DshError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<InstallStage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
    pub logs: Vec<LogLine>,
    /// 当前 dsh 页 URL(就绪过即携带;壳页 set iframe.src 用)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsh_url: Option<String>,
}

/// `dsh-url` 事件载荷(record_dsh_url 推 URL 给壳页,壳页 set iframe.src)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DshUrlView {
    pub url: String,
}

/// `dsh-exited` 事件载荷(reaper 推 dsh 意外退出给壳页,#31 场景 6 / #40)。
/// 只有「非主动退出流程」的意外退出才推(退出流程 / 升级流水线在途被抑制,
/// 见 spawn_reaper);壳页据此进入全屏错误覆盖层 + [重试]。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DshExitedView {
    /// 退出码(句柄异常等未知场景为 None)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

struct BootState {
    phase: Phase,
    error: Option<DshError>,
    logs: VecDeque<LogLine>,
    /// Node 检测结果(仅 checking 阶段携带;set_node_version 写入并推事件,
    /// 离开 checking 阶段清空——检测结果是 checking 阶段的呈现,不留到下一阶段)
    node_version: Option<String>,
    /// 安装模拟进度与子阶段(installing 期间由 emit_install_progress 维护,
    /// 快照挂载/重试时同步给前端;离开 installing 阶段清空)
    progress: Option<u8>,
    stage: Option<InstallStage>,
}

/// dsh 生命周期管理器。Clone 共享内部状态(boot 线程与 reaper 线程各持一份)。
#[derive(Clone)]
pub struct DshManager {
    app: AppHandle,
    state: Arc<Mutex<BootState>>,
    child: Arc<Mutex<Option<Child>>>,
    /// 安装中 npm 子进程 pid(退出收敛时一并杀掉;npm 会再拉起 node 子进程,按树杀)
    install_pid: Arc<Mutex<Option<u32>>>,
    /// 当前 dsh 页 URL(boot 就绪时记录;单一事实来源,record 即推壳页,#36)。
    dsh_url: Arc<Mutex<Option<String>>>,
    /// boot 流水线启动时刻(启动页耗时显示起点;重试覆盖为新一轮起点)
    started_at: Arc<Mutex<Option<Instant>>>,
}

impl DshManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            state: Arc::new(Mutex::new(BootState {
                phase: Phase::Idle,
                error: None,
                logs: VecDeque::new(),
                node_version: None,
                progress: None,
                stage: None,
            })),
            child: Arc::new(Mutex::new(None)),
            install_pid: Arc::new(Mutex::new(None)),
            dsh_url: Arc::new(Mutex::new(None)),
            started_at: Arc::new(Mutex::new(None)),
        }
    }

    /// 记录当前 dsh 页 URL 并推 `dsh-url` 事件给壳页(壳页 set iframe.src)。
    /// 单一事实来源 + 唯一推送口:4 个端口变化时点(boot 就绪 / 升级链就绪 /
    /// 升级卡「稍后/返回」重启 / 崩溃重试)统一收敛于此(ADR 0001 / #29,#36)。
    pub(crate) fn record_dsh_url(&self, url: String) {
        if let Ok(mut g) = self.dsh_url.lock() {
            *g = Some(url.clone());
        }
        let _ = self.app.emit_to("main", "dsh-url", DshUrlView { url });
    }

    /// 当前 boot 流水线阶段(service_status 的输入事实;reaper 的意外退出判定用)。
    fn phase(&self) -> Phase {
        self.state.lock().map(|s| s.phase).unwrap_or(Phase::Error)
    }

    /// 服务运行状态:阶段 × 子进程存活两事实的组合判定(语义见 derive_status)。
    /// boot_start 守卫 / 升级确认 / 升级卡恢复判断 / 手动检查 boot 就绪判定
    /// 统一经此谓词。
    pub(crate) fn service_status(&self) -> ServiceStatus {
        derive_status(self.phase(), self.child_alive())
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
        // 进度字段在状态内维护(见 set_install_progress_snapshot);其余照常
        let (progress, stage) = match s.phase {
            Phase::Installing => (s.progress, s.stage),
            _ => (None, None),
        };
        BootStateSnapshot {
            phase: s.phase,
            error: s.error.clone(),
            node_version: s.node_version.clone(),
            progress,
            stage,
            elapsed_secs: self.boot_elapsed_secs(),
            logs,
            dsh_url: dsh_url(self),
        }
    }

    /// 阶段迁移:更新状态并推送 `boot-state` 事件(阶段 + 耗时,不含日志)。
    /// emit_to("main"):只投递主窗口 webview,不广播给其它窗口。
    /// 事件同时带 elapsed_secs(从流水线启动起累计)——前端据此显示全程耗时。
    fn set_phase(&self, phase: Phase, error: Option<DshError>) {
        let mut node_version = None;
        if let Ok(mut s) = self.state.lock() {
            // 新一次 boot 的语义边界:进入 Checking 时清空上一轮的日志缓冲
            if phase == Phase::Checking {
                s.logs.clear();
            }
            s.phase = phase;
            s.error = error.clone();
            // 离开 installing:进度字段不再有意义,清空防快照残留
            if phase != Phase::Installing {
                s.progress = None;
                s.stage = None;
            }
            // 离开 checking:node 检测结果不再有意义,清空防快照残留
            // (检测结果是 checking 阶段的呈现,set_node_version 在阶段内写入)
            if phase != Phase::Checking {
                s.node_version = None;
            }
            node_version = s.node_version.clone();
        }
        match &error {
            Some(e) => log::error!("[dsh] phase → {phase:?}: {e:?}"),
            None => log::info!("[dsh] phase → {phase:?}"),
        }
        // 任务栏进度同步(Windows):checking/starting 流动动画、installing 确定进度起点
        // (进度随后由安装进度线程逐级更新)、ready/error 清除——窗口导航/错误页后
        // 不得残留旧进度。非安装阶段的事件不带 progress/stage 字段。
        match phase {
            Phase::Checking | Phase::Starting => self.set_taskbar_progress(TaskbarProgress::Indeterminate),
            Phase::Installing => self.set_taskbar_progress(TaskbarProgress::Percent(0)),
            _ => self.set_taskbar_progress(TaskbarProgress::Clear),
        }
        let _ = self
            .app
            .emit_to("main", "boot-state", BootStateView {
                phase,
                error,
                node_version,
                progress: None,
                stage: None,
                elapsed_secs: self.boot_elapsed_secs(),
            });
    }

    /// 写入 Node 检测结果并推送 checking 阶段事件(前端启动页显示
    /// 「检测到 Node.js vX」,让用户看到检测在推进——checking 阶段检测后
    /// 还要做 npm root -g 等检查,有可感知的展示窗口)。
    /// 由 boot_pipeline 在 check_node 成功后调用;阶段仍在 Checking。
    fn set_node_version(&self, ver: String) {
        if let Ok(mut s) = self.state.lock() {
            s.node_version = Some(ver.clone());
        }
        log::info!("[dsh] node 检测完成: {ver}");
        let _ = self
            .app
            .emit_to("main", "boot-state", BootStateView {
                phase: Phase::Checking,
                error: None,
                node_version: Some(ver),
                progress: None,
                stage: None,
                elapsed_secs: self.boot_elapsed_secs(),
            });
    }

    /// 记录流水线启动时刻(boot_start 时调用;重试会覆盖为新一轮起点)。
    /// 耗时显示的起点 = 真实流水线启动时刻,不依赖前端挂载时机(挂载可能晚于启动)。
    fn mark_boot_started(&self) {
        if let Ok(mut g) = self.started_at.lock() {
            *g = Some(Instant::now());
        }
    }

    /// 从流水线启动起的累计秒数(None = 尚未启动过)。
    fn boot_elapsed_secs(&self) -> Option<u64> {
        self.started_at
            .lock()
            .ok()
            .and_then(|g| g.map(|t| t.elapsed().as_secs()))
    }

    /// 安装进度事件:推 `boot-state` { phase: Installing, progress, stage, elapsed }。
    /// 纯视觉呈现,不参与任何业务决策;100% 只能由调用方(npm 进程退出)校准。
    fn emit_install_progress(&self, stage: InstallStage, progress: u8) {
        // 同步进度到状态:挂载/重试的快照能拿到当前值(进度线程是异步的)
        if let Ok(mut s) = self.state.lock() {
            s.progress = Some(progress);
            s.stage = Some(stage);
        }
        self.set_taskbar_progress(TaskbarProgress::Percent(progress));
        let _ = self
            .app
            .emit_to("main", "boot-state", BootStateView {
                phase: Phase::Installing,
                error: None,
                node_version: None,
                progress: Some(progress),
                stage: Some(stage),
                elapsed_secs: self.boot_elapsed_secs(),
            });
    }

    /// 启动安装模拟进度线程(纯视觉,与 npm 进程真实成败判定完全无关):
    /// 事件经 boot-state 推送;停表用 ProgressTicker::stop_and_join(见其文档)。
    fn start_install_progress(&self) -> npm::ProgressTicker {
        let m = self.clone();
        npm::ProgressTicker::start(move |stage, pct| m.emit_install_progress(stage, pct))
    }

    /// Windows 任务栏图标进度(tauri `set_progress_bar`,仅 Windows 生效;
    /// 其余平台 no-op——macOS 无任务栏进度、Linux 仅 libunity 桌面环境)。
    fn set_taskbar_progress(&self, state: TaskbarProgress) {
        #[cfg(windows)]
        {
            use tauri::window::{ProgressBarState, ProgressBarStatus};
            let Some(win) = self.app.get_webview_window("main") else {
                return;
            };
            let (status, progress) = match state {
                TaskbarProgress::Clear => (ProgressBarStatus::None, None),
                TaskbarProgress::Indeterminate => (ProgressBarStatus::Indeterminate, None),
                TaskbarProgress::Percent(p) => (ProgressBarStatus::Normal, Some(p.into())),
            };
            let _ = win.set_progress_bar(ProgressBarState {
                status: Some(status),
                progress,
            });
        }
        #[cfg(not(windows))]
        {
            let _ = state; // 平台守卫:非 Windows 不调用(避免无意义开销)
        }
    }

    fn set_error(&self, error: DshError) {
        self.set_phase(Phase::Error, Some(error));
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
                s.logs.push_back(LogLine {
                    stream: stream.into(),
                    line: trimmed.to_string(),
                });
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

    /// dsh 子进程是否存活:句柄在且 try_wait 未退出。try_wait 出错(句柄异常/
    /// 已被他处收割)按已退出处理(保守,允许重跑);锁不可用(中毒)按不存活。
    fn child_alive(&self) -> bool {
        let Ok(mut guard) = self.child.lock() else {
            return false;
        };
        let Some(child) = guard.as_mut() else {
            return false;
        };
        matches!(child.try_wait(), Ok(None))
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

/// 安装过程观察者实现(npm::install_global 的 seam):pid 登记随退出收敛
/// 按树杀;日志行入环形缓冲(boot 阶段落盘,见 push_log)。
impl npm::InstallObserver for DshManager {
    fn install_pid(&self, pid: Option<u32>) {
        match pid {
            Some(p) => self.set_install_pid(p),
            None => self.clear_install_pid(),
        }
    }

    fn install_log(&self, stream: &str, line: &str) {
        self.push_log(stream, line.to_string());
    }
}

// ── dsh 进程与导航 ─────────────────────────────────────────────────

/// 当前 dsh 页 URL(None = 尚未就绪过)。升级卡片「稍后/返回」的导航目标。
pub fn dsh_url(manager: &DshManager) -> Option<String> {
    manager
        .dsh_url
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

/// dsh 服务 URL 拼接(单一事实来源:boot 就绪导航 / 升级链就绪导航 / 返回 dsh 共用)。
pub(crate) fn dsh_url_for_port(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// 杀掉 dsh 子进程(进程树)与安装中的 npm 进程,幂等。
/// wait 有界(任务失败/被杀时 wait 会挂死,退出路径不能无上限等待):
/// 超时再补杀一次,仍不退出只记日志(升级链用 kill_child_confirm 感知结果)。
pub fn kill_child(manager: &DshManager) {
    let _ = kill_child_inner(manager);
}

/// 杀掉 dsh 子进程并确认其已退出(升级链 killing 阶段,#3 §2)。
/// 返回 false = 杀后仍在运行(升级报 UpgradeKillFailed;进程句柄放回 manager,
/// 后续重试/退出收敛还能再杀)。没有 dsh 在跑(句柄已空)按成功处理——
/// 升级的目标状态「dsh 不在运行」已达成。不杀 npm 安装进程:升级时无 boot
/// 安装在途,install_pid 归退出收敛路径统一处理。
pub fn kill_child_confirm(manager: &DshManager) -> bool {
    match kill_child_inner(manager) {
        ConfirmResult::Killed | ConfirmResult::AlreadyGone => true,
        ConfirmResult::StillRunning => false,
    }
}

enum ConfirmResult {
    Killed,
    AlreadyGone,
    StillRunning,
}

fn kill_child_inner(manager: &DshManager) -> ConfirmResult {
    let Some(mut child) = manager.take_child() else {
        // 安装中的 npm 进程仍按退出收敛语义杀(幂等)
        if let Some(pid) = manager.take_install_pid() {
            log::info!("[dsh] kill npm 安装进程 pid={pid}");
            kill_pid_tree(pid);
        }
        return ConfirmResult::AlreadyGone;
    };
    log::info!("[dsh] kill dsh 子进程 pid={}", child.id());
    for _ in 0..2 {
        kill_pid_tree(child.id());
        match wait_with_timeout(&mut child, KILL_CONFIRM_TIMEOUT) {
            Ok(_) => {
                if let Some(pid) = manager.take_install_pid() {
                    log::info!("[dsh] kill npm 安装进程 pid={pid}");
                    kill_pid_tree(pid);
                }
                return ConfirmResult::Killed;
            }
            Err(_) => continue, // 补杀一次
        }
    }
    log::error!("[dsh] dsh 子进程 pid={} 杀后仍未退出", child.id());
    // 进程仍活着:句柄放回 manager,后续重试 / 退出收敛还能再杀
    manager.set_child(child);
    if let Some(pid) = manager.take_install_pid() {
        log::info!("[dsh] kill npm 安装进程 pid={pid}");
        kill_pid_tree(pid);
    }
    ConfirmResult::StillRunning
}

/// 杀后确认等待上限(taskkill /T /F 后一般毫秒级退出;3s 未退视为杀失败)。
/// 秒数值另导出:升级链的 UpgradeKillFailed 错误 detail 携带同一事实。
pub const KILL_CONFIRM_TIMEOUT_SECS: u64 = 3;
const KILL_CONFIRM_TIMEOUT: Duration = Duration::from_secs(KILL_CONFIRM_TIMEOUT_SECS);

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

/// npm 全局安装 dsh(boot 传 "@latest" 跟随 latest;dsh 升级链传 "@<pin>" 精确版本,#3 §7)。
/// 安装流程本体(执行/超时/孤儿回收/ETARGET 回退/错误分类)在 npm::install_global
/// (npm 域单一事实来源);本类型实现 npm::InstallObserver——pid 登记(升级期间
/// 用户退出时随退出收敛一并杀)与日志行入环形缓冲。安装包内置离线缓存存在时
/// 优先离线安装(命中秒级完成、缺失回退网络)。
pub(crate) fn npm_install_global(manager: &DshManager, version_spec: &str) -> Result<(), DshError> {
    let cache_dir = manager
        .app
        .path()
        .resource_dir()
        .ok()
        .as_deref()
        .and_then(npm::bundle_cache_dir);
    npm::install_global(manager, cache_dir.as_deref(), version_spec)
}

/// 启动 `node <bin.js> web --port 0`,返回 stdout/stderr 合流后的行接收端
/// (boot 与升级链复用;升级链用同一 bin 路径,升级前后不变,#2 调研)
pub(crate) fn spawn_dsh(
    manager: &DshManager,
    bin: &Path,
) -> Result<Receiver<(String, String)>, DshError> {
    let mut binding = Command::new("node");
    let cmd = new_process_group(no_window(&mut binding))
        .arg(bin)
        .args(["web", "--port", "0"])
        .env("DSH_TELEMETRY_DISABLED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| DshError::DshSpawnFailed { detail: e.to_string() })?;

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
    let digits: String = port_part
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
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
/// boot 与升级链复用。
pub(crate) fn wait_ready(
    manager: &DshManager,
    rx: Receiver<(String, String)>,
) -> Result<(u16, Receiver<(String, String)>), DshError> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok((stream, line)) => {
                manager.push_log(&stream, line.clone());
                if let Some(port) = parse_ready_line(&line) {
                    if tcp_wait(port) {
                        return Ok((port, rx));
                    }
                    return Err(DshError::ReadyPortUnavailable { port });
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // 子进程是否已退出(锁作用域内完成判断,避免守卫借用逃逸)
                if let Some(code) = child_exit_code(manager) {
                    return Err(DshError::DshExitedEarly { exit_code: code });
                }
                if Instant::now() >= deadline {
                    return Err(DshError::DshStartTimeout {
                        seconds: START_TIMEOUT.as_secs(),
                    });
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // 输出流关闭 = 读线程结束 = 子进程已退出(或句柄被继承导致异常断流)
                return match child_exit_code(manager) {
                    Some(code) => Err(DshError::DshExitedEarly { exit_code: code }),
                    None => Err(DshError::DshExitedEarlyNoCode),
                };
            }
        }
    }
}

// ── 启动 dsh 服务的六步时序(单一事实来源)─────────────────────────

/// 启动 dsh 服务并等待就绪(boot / 升级链 / 升级卡「稍后/返回」共用,时序单点持有):
/// spawn → wait_ready(就绪信号 = stdout 打印 READY_PREFIX)→ 帧嵌入防御检查
/// (ADR 0001 核心假设)→ record_dsh_url(URL 单一事实来源,推给壳页)→ 交 reaper
/// (持续排空输出流,防 64KB 管道阻塞挂死)。任一步失败:按进程树 kill 清理后返回
/// 错误,不残留半启动进程。
/// `suppress_exit_report`:升级链传 true——成功后清除 UPGRADE_ACTIVE 抑制标志
/// (旧 dsh 的 reaper 判定点早已过,清除安全;#3 §2);boot / 恢复服务传 false。
/// 返回已就绪端口(调用方仅用于日志/呈现)。
pub(crate) fn start_service(
    manager: &DshManager,
    bin: &Path,
    suppress_exit_report: bool,
) -> Result<u16, DshError> {
    let rx = spawn_dsh(manager, bin)?;
    let (port, rx) = match wait_ready(manager, rx) {
        Ok(p) => p,
        Err(e) => {
            kill_child(manager);
            return Err(e);
        }
    };
    let url = dsh_url_for_port(port);
    // 就绪确认的一环:帧嵌入回归检查(命中 XFO / frame-ancestors = iframe 架构
    // 无法呈现该版本,按启动失败处理,错误指引回退预案,见 DshError::FrameBlocked)
    if let Err(e) = tauri::async_runtime::block_on(check_frame_blocking(&url)) {
        kill_child(manager);
        return Err(e);
    }
    manager.record_dsh_url(url.clone());
    spawn_reaper(manager.clone(), rx);
    if suppress_exit_report {
        set_upgrade_active(false);
    }
    log::info!("[dsh] start_service: 就绪,推 URL 给壳页 → {url}");
    Ok(port)
}

/// CSP 头是否含 frame-ancestors 指令(纯函数,可测)。
/// 按指令解析(分号分段,取每段首 token 与指令名比对,大小写不敏感)——
/// 不按子串匹配,避免 nonce/值里恰好出现同名串的误报。
fn csp_blocks_framing(csp_value: &str) -> bool {
    csp_value.split(';').any(|directive| {
        let mut parts = directive.split_whitespace();
        parts
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
    })
}

/// 判定响应头是否禁止跨源 iframe 嵌入(纯函数,可测)。命中返回头原文
/// (供错误展示,前端模板插值);未命中返回 None。
///
/// 判据(壳页是本地 origin、dsh 是 127.0.0.1 动态端口,必然跨源):
/// - 任意 X-Frame-Options 头即禁止——DENY 自不必说,SAMEORIGIN 对跨源
///   同样拦截(HeaderMap 取值大小写不敏感);
/// - CSP 含 frame-ancestors 指令即禁止——dsh 服务无从枚举壳页 origin,
///   出现即几乎必然不含壳页、必拦。
fn frame_blocking_header(headers: &reqwest::header::HeaderMap) -> Option<String> {
    if let Some(xfo) = headers.get(reqwest::header::X_FRAME_OPTIONS) {
        let value = String::from_utf8_lossy(xfo.as_bytes()).to_string();
        return Some(format!("X-Frame-Options: {value}"));
    }
    for csp in headers.get_all(reqwest::header::CONTENT_SECURITY_POLICY) {
        let value = String::from_utf8_lossy(csp.as_bytes());
        if csp_blocks_framing(&value) {
            return Some(format!("Content-Security-Policy: {value}"));
        }
    }
    None
}

/// 对运行中的 dsh 服务做帧嵌入回归检查(GET 根路径,读响应头)。
/// 命中 XFO / frame-ancestors → Err(FrameBlocked,start_service 按启动失败处理,
/// 前端按 errors.UpgradeFrameBlocked 翻译并指引回退预案)。
/// 请求失败/超时/客户端构造失败 → 记日志放行:探测不确定不等于「被禁止」,
/// 不为不确定的探测拦掉启动(防御检查是找「已确认的上游耦合」,不是网络
/// 可用性检查;wait_ready 已确认服务在监听)。
async fn check_frame_blocking(url: &str) -> Result<(), DshError> {
    ensure_tls_provider(); // 与 fetch_latest_version 同款:本地请求也经 rustls 客户端
    let client = reqwest::Client::builder()
        .timeout(FRAME_CHECK_TIMEOUT)
        .build()
        .ok();
    let Some(client) = client else {
        return Ok(());
    };
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[dsh] 帧嵌入回归检查:请求失败(按未命中放行) {url}: {e}");
            return Ok(());
        }
    };
    if let Some(header) = frame_blocking_header(resp.headers()) {
        log::error!("[dsh] 帧嵌入回归检查:命中 {header} → 启动失败");
        return Err(DshError::FrameBlocked { header });
    }
    log::info!("[dsh] 帧嵌入回归检查通过(无 XFO / frame-ancestors) {url}");
    Ok(())
}

/// rustls ring provider(与 updater 插件同款):插件在其自身路径懒安装,
/// 本模块与 upgrade.rs(registry 直查)的请求可能在它之前发生(启动并发),
/// 显式安装保证确定性(幂等)。
pub(crate) fn ensure_tls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

// ── boot 流水线(运行在工作线程上)─────────────────────────────────

fn boot_pipeline(manager: &DshManager) {
    // 1. 环境检查
    log::info!("[dsh] boot: checking node…");
    manager.set_phase(Phase::Checking, None);
    let node_version = match npm::check_node() {
        Ok(v) => v,
        Err(e) => {
            manager.set_error(e);
            return;
        }
    };
    // 检测结果可视化:checking 阶段推版本信息,启动页显示「检测到 Node.js vX」
    manager.set_node_version(node_version);

    // 2. 全局 dsh 检测(完整性校验:bin.js 存在,不只版本号)
    //    「有则用」:全局已有 dsh(任意可用版本)直接用,不重装不强制升级
    let bin = match npm::global_dsh_bin() {
        Some(b) => b,
        None => {
            log::info!("[dsh] boot: global dsh not found, installing…");
            manager.set_phase(Phase::Installing, None);
            // 进度模拟(纯视觉):0% 起点 → 进度线程按真实时间推进(封顶 99%)。
            // 锚点 = npm 进程退出:成功 → 校准 100%;失败/超时 → 不校准,
            // 直接进错误页(模拟永不领先于真实结果,不会出现「100% 却失败」)。
            // 停表后 join:线程在途事件先于 100% 校准/错误事件送达(见
            // ProgressTicker 的说明),join 后进度字段不再被线程改写。
            manager.emit_install_progress(InstallStage::Fetching, 0);
            let progress_thread = manager.start_install_progress();
            let result = npm_install_global(manager, "@latest");
            progress_thread.stop_and_join();
            if let Err(e) = result {
                manager.set_error(e);
                return;
            }
            manager.emit_install_progress(InstallStage::Finishing, 100);
            // 安装后完整性复检:装完仍然不可用视为失败
            match npm::global_dsh_bin() {
                Some(b) => b,
                None => {
                    manager.set_error(DshError::InstallVerifyFailed);
                    return;
                }
            }
        }
    };

    // 3. 启动 dsh web 并等待就绪:六步时序(spawn → wait_ready → 帧嵌入检查
    //    → record_dsh_url → 交 reaper)由 start_service 单点持有,此处只收结果。
    //    就绪后壳页 set iframe.src(前端 boot 浮层的退出过渡动画由 URL 到达
    //    触发,useBootExit,fallback 兜底,动画不阻塞呈现)。
    log::info!("[dsh] boot: starting dsh web…");
    manager.set_phase(Phase::Starting, None);
    match start_service(manager, &bin, false) {
        Ok(port) => {
            log::info!("[dsh] boot: ready on port {port}");
            manager.set_phase(Phase::Ready, None);
        }
        Err(e) => manager.set_error(e),
    }
}

/// 收割线程:排空 channel 直到读线程结束(进程退出或被杀),再 wait 回收。
/// dsh 意外退出(非主动退出流程)时推 `dsh-exited` 事件:壳页全屏错误覆盖层
/// + [重试](重跑 boot)。原原生弹窗已在 #39 移除,呈现转本覆盖层(#31 场景 6 / #32 拍板 / #40 施工)。
///
/// 意外退出判定与 #32「以 upgrade pipeline 在途标志为准」一致。
/// - is_quitting:程序化退出流程(quit_app / 托盘退出),不推
/// - upgrade_active:升级流水线杀旧 dsh 是流水线的一部分(killing 阶段),不推
/// - 其余(phase 仍为 Ready 而 dsh 进程退出)= 意外退出,推事件
///
/// 竞态防护:只有 phase 仍为 Ready 时才取子进程句柄。若排空期间用户已重试
/// (phase 进入 Checking),新 boot 的 child 已写入 manager——此时取走会令
/// 新 boot 的 wait_ready 误判"进程提前退出",故直接放弃收割。
pub(crate) fn spawn_reaper(manager: DshManager, rx: Receiver<(String, String)>) {
    thread::spawn(move || {
        // 持续排空(丢弃);读线程随子进程退出而结束,tx 全部 drop 后 recv 返回 Err
        while rx.recv().is_ok() {}

        if manager.phase() != Phase::Ready {
            return;
        }
        let child = manager.take_child();
        if let Some(mut c) = child {
            let status = c.wait();
            let exit_code = status.ok().and_then(|s| s.code());
            let quitting = is_quitting();
            let upgrade = upgrade_active();
            log::info!(
                "[dsh] reaper: dsh 子进程退出, exit_code={exit_code:?}, quitting={quitting}, upgrade_active={upgrade}"
            );
            if !quitting && !upgrade {
                let _ = manager
                    .app
                    .emit_to("main", "dsh-exited", DshExitedView { exit_code });
            }
        }
    });
}

// ── Tauri commands ─────────────────────────────────────────────────

/// 触发(或重试)boot 流水线,并返回含最近日志的状态快照。
/// 挂载/重试一调两用:触发 + 拉快照,命令面从 3 收窄到 2(boot/quit_app)。
/// phase 守卫:非 Idle/Error 且 dsh 在跑时 no-op(防 StrictMode 双 invoke /
/// 正常态误触重启;Ready + dsh 已死的意外退出重试放行,见 boot_start),
/// 快照仍正常返回。
/// async + Result:不占用 IPC/主线程(tauri 2 要求含引用输入的 async command 返回 Result)。
#[tauri::command]
pub async fn boot(state: tauri::State<'_, DshManager>) -> Result<BootStateSnapshot, String> {
    boot_start(state.inner());
    Ok(state.inner().snapshot())
}

/// 在独立线程启动流水线;Idle(首启)/ Error(重试)/ Ready 且 dsh 已死(意外
/// 退出覆盖层的 [重试] 重跑 boot,#40)时才生效,其余阶段 no-op。
/// BOOTING 标志防并发双流水线(StrictMode 双 invoke + setup 主动启动)。
pub fn boot_start(manager: &DshManager) {
    // 可重跑判定(service_status 的第一个消费者):Idle 首启 / Error 重试 /
    // DeadAfterCrash(就绪后意外退出,reaper 已收割)才生效,其余 no-op——
    // Running 的正常态下 boot 命令 no-op,防误触重启
    let can_boot = matches!(
        manager.service_status(),
        ServiceStatus::NotReady | ServiceStatus::Error | ServiceStatus::DeadAfterCrash
    );
    if !can_boot {
        return;
    }
    let phase = manager.phase();
    if BOOTING.swap(true, Ordering::SeqCst) {
        return;
    }
    // 耗时显示起点:真实流水线启动时刻(重试会覆盖为新一轮起点)
    manager.mark_boot_started();
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
            m.set_error(DshError::Internal { message: msg });
        }
    });
}

/// 程序化退出(所有退出路径统一收敛,调用方只需持有 app):置 QUITTING 标志
/// 放行 CloseRequested(不再弹关闭询问)→ 杀 dsh 子进程(幂等)→ exit(0)。
/// 装配细节不外泄;退出收敛的最终防线仍是 lib.rs 的 ExitRequested 兜底再杀一次。
pub(crate) fn shutdown_and_exit(app: &AppHandle) {
    set_quitting();
    if let Some(m) = app.try_state::<DshManager>() {
        kill_child(m.inner());
    }
    app.exit(0);
}

/// 退出应用:杀子进程 + exit(0)。程序化退出先置 QUITTING 标志放行 CloseRequested。
#[tauri::command]
pub async fn quit_app(app: tauri::AppHandle) -> Result<(), String> {
    shutdown_and_exit(&app);
    Ok(())
}

// ── 测试 ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_status_maps_phase_times_alive_exhaustively() {
        // service_status 的组合语义穷举(谓词是五处调用点的唯一判定来源):
        // - 只有 Ready 关注子进程死活:「Ready + 句柄空 = 意外退出待重跑」
        //   (reaper 已收割,DeadAfterCrash 让 boot_start 放行重跑)
        // - 流水线在途一律 Booting,与子进程是否已 spawn 无关
        // - Idle/Error 与子进程事实无关(错误态不可能有存活进程)
        for alive in [false, true] {
            assert_eq!(derive_status(Phase::Idle, alive), ServiceStatus::NotReady);
            assert_eq!(derive_status(Phase::Error, alive), ServiceStatus::Error);
            for phase in [Phase::Checking, Phase::Installing, Phase::Starting] {
                assert_eq!(derive_status(phase, alive), ServiceStatus::Booting);
            }
            assert_eq!(
                derive_status(Phase::Ready, alive),
                if alive {
                    ServiceStatus::Running
                } else {
                    ServiceStatus::DeadAfterCrash
                }
            );
        }
    }

    #[test]
    fn boot_ready_semantics_match_phase_ready() {
        // 不变量:升级确认守卫与手动检查 boot 就绪判定用的「Ready 生死两态」,
        // 与旧 phase()==Ready 的判定等价(Ready 的两种 ServiceStatus 恰是
        // 它按子进程死活二分的结果)
        let ready_like = [ServiceStatus::Running, ServiceStatus::DeadAfterCrash];
        let not_ready_like = [
            ServiceStatus::NotReady,
            ServiceStatus::Booting,
            ServiceStatus::Error,
        ];
        for s in ready_like {
            assert!(matches!(s, ServiceStatus::Running | ServiceStatus::DeadAfterCrash));
        }
        for s in not_ready_like {
            assert!(!matches!(s, ServiceStatus::Running | ServiceStatus::DeadAfterCrash));
        }
        // can_boot 的补集:NotReady/Error/DeadAfterCrash 三态可重跑,
        // Running/Booting 一律 no-op
        for s in [ServiceStatus::NotReady, ServiceStatus::Error, ServiceStatus::DeadAfterCrash] {
            assert!(matches!(s, ServiceStatus::NotReady | ServiceStatus::Error | ServiceStatus::DeadAfterCrash));
        }
        for s in [ServiceStatus::Running, ServiceStatus::Booting] {
            assert!(!matches!(s, ServiceStatus::NotReady | ServiceStatus::Error | ServiceStatus::DeadAfterCrash));
        }
    }

    #[test]
    fn parse_ready_line_extracts_port() {
        assert_eq!(
            parse_ready_line("dsh web: http://127.0.0.1:3080"),
            Some(3080)
        );
        // 尾随斜杠 / 空白
        assert_eq!(
            parse_ready_line("dsh web: http://127.0.0.1:3080/"),
            Some(3080)
        );
        assert_eq!(
            parse_ready_line("dsh web: http://127.0.0.1:3080   "),
            Some(3080)
        );
        // 尾随路径/查询串:取数字前缀
        assert_eq!(
            parse_ready_line("dsh web: http://127.0.0.1:3080/?x=1"),
            Some(3080)
        );
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

    #[test]
    fn dsh_url_for_port_builds_loopback_url() {
        // 单一事实来源:boot / 升级链 / 返回 dsh 的导航目标共用同一拼接
        assert_eq!(dsh_url_for_port(3080), "http://127.0.0.1:3080");
        assert_eq!(dsh_url_for_port(0), "http://127.0.0.1:0");
    }

    // ── 帧嵌入回归检查(上游耦合防线,ADR 0001 / #41,start_service 就绪确认)──

    fn headers(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut m = reqwest::header::HeaderMap::new();
        for (k, v) in pairs {
            m.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        m
    }

    #[test]
    fn frame_blocking_header_detects_xfo() {
        // X-Frame-Options 任一取值都拦(SAMEORIGIN 对跨源同样拦截)
        for v in ["DENY", "SAMEORIGIN"] {
            let h = headers(&[("x-frame-options", v)]); // 头名大小写不敏感
            assert_eq!(
                frame_blocking_header(&h),
                Some(format!("X-Frame-Options: {v}")),
                "XFO={v} 必须命中"
            );
        }
        // 无帧头 → None
        assert_eq!(frame_blocking_header(&headers(&[])), None);
        assert_eq!(
            frame_blocking_header(&headers(&[("content-type", "text/html")])),
            None
        );
    }

    #[test]
    fn frame_blocking_header_detects_csp_frame_ancestors() {
        // frame-ancestors 指令(大小写不敏感)命中
        let h = headers(&[("content-security-policy", "default-src 'self'; frame-ancestors 'none'")]);
        assert_eq!(
            frame_blocking_header(&h),
            Some("Content-Security-Policy: default-src 'self'; frame-ancestors 'none'".to_string())
        );
        // 指令名大小写变体
        assert!(csp_blocks_framing("default-src 'self'; Frame-Ancestors https://x.com"));
        // 无 frame-ancestors 的 CSP 不命中(含其它安全指令)
        let h = headers(&[(
            "content-security-policy",
            "default-src 'self'; script-src 'nonce-frame-ancestors-x'",
        )]);
        assert_eq!(frame_blocking_header(&h), None);
        // 值里恰好出现同名串不误报(按指令解析,不按子串)
        assert!(!csp_blocks_framing("script-src 'nonce-frame-ancestors'"));
        // 多 CSP 头:任一命中即命中
        let h = headers(&[
            ("content-security-policy", "default-src 'self'"),
            ("content-security-policy", "frame-ancestors 'none'"),
        ]);
        assert_eq!(frame_blocking_header(&h).unwrap(), "Content-Security-Policy: frame-ancestors 'none'");
        // 空值/畸形值不 panic 不命中
        assert!(!csp_blocks_framing(""));
        assert!(!csp_blocks_framing(";;;"));
    }

    #[test]
    fn frame_blocking_header_prefers_xfo() {
        // 两者都命中时报告 XFO(先到先报告,判据一致性)
        let h = headers(&[
            ("x-frame-options", "DENY"),
            ("content-security-policy", "frame-ancestors 'none'"),
        ]);
        assert_eq!(frame_blocking_header(&h).unwrap(), "X-Frame-Options: DENY");
    }

    #[test]
    fn check_frame_blocking_roundtrip_with_local_server() {
        // 验收(issue #41):本地起一个带 XFO 头的假 dsh 服务,检查能报错。
        // 用 std TcpListener 起一次性 HTTP 响应(无外部依赖,单测内闭环)。
        fn serve_once(resp: String) -> (String, std::thread::JoinHandle<()>) {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = std::thread::spawn(move || {
                if let Ok((mut s, _)) = listener.accept() {
                    // 标准 HTTP 顺序:先读请求再回响应;响应发完保持连接片刻,
                    // 让客户端完整读完(读请求后立即回包 + 马上关会有 RST 竞态)
                    let mut buf = [0u8; 1024];
                    let _ = std::io::Read::read(&mut s, &mut buf);
                    let _ = std::io::Write::write_all(&mut s, resp.as_bytes());
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            });
            (format!("http://{addr}"), handle)
        }

        // 带 X-Frame-Options: DENY 的假服务 → 检查报 FrameBlocked
        let (url, h) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Frame-Options: DENY\r\nContent-Length: 4\r\n\r\n<h1>x</h1>"
                .to_string(),
        );
        let err = tauri::async_runtime::block_on(check_frame_blocking(&url)).unwrap_err();
        assert_eq!(
            err,
            DshError::FrameBlocked {
                header: "X-Frame-Options: DENY".into()
            }
        );
        h.join().unwrap();

        // 无帧头的假服务 → 放行
        let (url, h) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 4\r\n\r\n<h1>x</h1>"
                .to_string(),
        );
        tauri::async_runtime::block_on(check_frame_blocking(&url)).unwrap();
        h.join().unwrap();

        // 服务不存在(连接拒绝)→ 探测不确定,放行
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        tauri::async_runtime::block_on(check_frame_blocking(&format!("http://127.0.0.1:{port}")))
            .unwrap();
    }

    #[test]
    fn boot_state_view_omits_progress_fields_when_not_installing() {
        // 线上契约:非安装阶段的事件/快照不带 progress/stage 字段
        // (skip_serializing_if),前端据此区分确定/不确定进度
        let view = BootStateView {
            phase: Phase::Checking,
            error: None,
            node_version: None,
            progress: None,
            stage: None,
            elapsed_secs: Some(3),
        };
        let v = serde_json::to_value(&view).unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "phase": "checking", "elapsedSecs": 3 })
        );
        assert!(v.get("node_version").is_none());
        assert!(v.get("progress").is_none());
        assert!(v.get("stage").is_none());
    }

    #[test]
    fn boot_state_view_serializes_node_version_in_checking() {
        // 线上契约:checking 阶段检测完成携带 nodeVersion(前端「检测到 Node.js vX」);
        // 其余阶段不携带(离开 checking 清空,见 set_phase)
        let view = BootStateView {
            phase: Phase::Checking,
            error: None,
            node_version: Some("v22.19.0".into()),
            progress: None,
            stage: None,
            elapsed_secs: Some(1),
        };
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "phase": "checking",
                "nodeVersion": "v22.19.0",
                "elapsedSecs": 1
            })
        );
    }

    #[test]
    fn dsh_exited_view_serializes_exit_code() {
        // 线上契约(dsh-exited 事件):camelCase;退出码未知时字段缺省不出现
        assert_eq!(
            serde_json::to_value(DshExitedView { exit_code: Some(1) }).unwrap(),
            serde_json::json!({ "exitCode": 1 })
        );
        assert_eq!(
            serde_json::to_value(DshExitedView { exit_code: None }).unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn boot_state_view_serializes_install_progress_fields() {
        // 安装中:progress/stage 以 camelCase/小写形态进 payload,前端直接消费
        let view = BootStateView {
            phase: Phase::Installing,
            error: None,
            node_version: None,
            progress: Some(62),
            stage: Some(InstallStage::Reifying),
            elapsed_secs: Some(35),
        };
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::json!({
                "phase": "installing",
                "progress": 62,
                "stage": "reifying",
                "elapsedSecs": 35
            })
        );
    }
}
