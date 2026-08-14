//! dsh(DeepSeek Harness)子进程管理与启动流水线。
//!
//! 生命周期:checking(环境检查)→ installing(npm 安装)→ starting(启动 dsh web)
//! → ready(服务就绪,窗口导航到 dsh Web UI)。
//! 状态迁移经 `boot-state` 事件推给前端(只 emit 到 `main` 窗口);日志不推流,
//! 只入环形缓冲(异常时附在错误页)。
//!
//! IPC 命令面(最小化,2 个):
//! - `boot`(触发/重试流水线 + 返回含日志的当前状态快照;挂载时一调两用)
//! - `quit_app`(程序化退出:杀子进程 + exit)
//!
//! 安全语义(tauri 2.11.5 源码确认):
//! - 窗口 navigate 到 http://127.0.0.1:<port> 后,该页面是 remote origin:
//!   ACL 按 capability(local-only)拒绝其调用任何命令/监听事件/使用窗口 API;
//!   Tauri 的 app CSP 只注入资产协议提供的本地页面,dsh 页面的 CSP 归 dsh 服务器自身。
//!
//! 生产日志:本模块不直接 eprintln,统一走 `log` crate 宏(logging::init 落盘到
//! `<app_data_dir>/logs/app.log`,panic 经 hook 同落盘)。
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
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

/// dsh 要求的 Node 版本(仓库根 package.json engines):^22.19 || >=24。
/// 作为 NodeVersionUnmet 的结构化数据传给前端(版本规格是技术串,语言中立,
/// 保持英文形态以免 zh/en 两处维护同一规格)。
const NODE_REQ: &str = "Node.js ^22.19 or >=24";
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
/// 安装包内置 npm 离线缓存的相对目录名(位于 Tauri 资源目录下)。
/// 约定由本文件与 #6(CI 发版打包)共同持有:CI 把发版时的 dsh 依赖树提前
/// 下载成 npm cacache 打进安装包,本模块只消费、不校验内容。
const BUNDLE_CACHE_REL: &str = "npm-cache";
/// 日志环形缓冲容量(仅供异常时附上下文,不推流)
const LOG_CAP: usize = 200;

// ── 全局守卫(跨线程)────────────────────────────────────────────────

static QUITTING: AtomicBool = AtomicBool::new(false);
static DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);
/// 流水线运行中标志:防 StrictMode 双 invoke / setup 与前端同时触发导致双流水线竞态
static BOOTING: AtomicBool = AtomicBool::new(false);
/// dsh 升级链主动杀旧 dsh 时的抑制标志(#3 §2):独立于 set_quitting——
/// 升级不退出应用、不要求放行 CloseRequested(关闭三选对话框整个会话保持有效),
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// "stdout" | "stderr"
    pub stream: String,
    pub line: String,
}

/// 安装模拟进度的子阶段。npm 安装期**没有真实百分比**(管道非 TTY + `--no-progress`,
/// 输出块缓冲突发到达,调研 #2 实测),本枚举是时间驱动的语义分段,供进度文案与
/// 事件携带;boot 安装与 dsh 升级链(未落地)复用同一模拟逻辑(install_progress_at)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum InstallStage {
    /// 下载依赖
    Fetching,
    /// 依赖解包写入安装目录
    Reifying,
    /// 收尾(接近完成)
    Finishing,
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
    pub error: Option<BootError>,
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

/// `boot` 命令返回的状态快照:含最近日志与当前进度(挂载/重试一调两用)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootStateSnapshot {
    pub phase: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<BootError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<InstallStage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
    pub logs: Vec<LogLine>,
}

/// 结构化失败原因(kind + data,serde tag/content 序列化为
/// `{"kind":"NodeCheckTimeout","data":{"seconds":10}}`,unit 变体无 data 字段)。
/// 前端经 toStructuredError 归约、渲染时按 `errors.<kind>` 键翻译
/// (见 src/lib/error.ts 与 src/locales/*.json)——错误串不在此处拼装,
/// 数据只携带运行时事实(超时秒数/退出码/版本/stderr 原文),文案模板在 locale JSON。
///
/// NodeMissing/NodeVersionUnmet 两个 kind 走 Node 引导页(展示要求 + 当前检测结果
/// + 官网下载/重试,见前端 isNodeGuideError);其余错误留通用错误页。
/// 版本规格(required)只由本文件 NODE_REQ 持有,随错误数据传给前端渲染——
/// 前端不复制规格文本,避免 zh/en 与 Rust 三处维护同一串。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "PascalCase", rename_all_fields = "camelCase")]
pub enum BootError {
    /// 未检测到 Node.js(带版本要求,供引导页展示)
    NodeMissing { required: String },
    /// `node --version` 检查超时
    NodeCheckTimeout { seconds: u64 },
    /// `node --version` 进程 IO 失败
    NodeCheckFailed { detail: String },
    /// `node --version` 非零退出
    NodeVersionCheckFailed { exit_code: i32, detail: String },
    /// 无法解析 node 版本号
    NodeVersionParseFailed { version: String },
    /// 版本不满足 ^22.19 || >=24
    NodeVersionUnmet { current: String, required: String },
    /// 无法执行 npm(未安装/不可用)
    NpmRootSpawnFailed,
    /// `npm root -g` 超时
    NpmRootTimeout { seconds: u64 },
    /// `npm root -g` 进程 IO 失败
    NpmRootIoFailed { detail: String },
    /// `npm root -g` 非零退出
    NpmRootExitFailed { exit_code: i32, detail: String },
    /// `npm root -g` 输出为空
    NpmRootEmpty,
    /// 无法启动 npm 安装进程
    NpmSpawnFailed { detail: String },
    /// 安装失败:权限类(EPERM/EACCES),带退出码
    InstallFailedPermission { exit_code: i32, stderr_tail: String },
    /// 安装失败:权限类,无退出码(异常退出)
    InstallFailedPermissionAbnormal { stderr_tail: String },
    /// 安装失败:非权限类(网络等),带退出码
    InstallFailedNetwork { exit_code: i32, stderr_tail: String },
    /// 安装失败:非权限类,无退出码(异常退出)
    InstallFailedNetworkAbnormal { stderr_tail: String },
    /// 安装超时
    InstallTimeout { seconds: u64 },
    /// 安装进程 IO 异常
    NpmInstallIoFailed { detail: String },
    /// 安装后完整性复检失败
    InstallVerifyFailed,
    /// 无法启动 dsh 进程
    DshSpawnFailed { detail: String },
    /// 就绪行已打印但端口未监听
    ReadyPortUnavailable { port: u16 },
    /// 进程提前退出,已知退出码
    DshExitedEarly { exit_code: i32 },
    /// 进程提前退出,无退出码(句柄缺失等)
    DshExitedEarlyNoCode,
    /// 启动超时(未收到就绪信号)
    DshStartTimeout { seconds: u64 },
    /// 无法导航窗口到 dsh 页面
    NavigateFailed,
    /// 流水线内部 panic 等未知内部错误
    Internal { message: String },
}

struct BootState {
    phase: Phase,
    error: Option<BootError>,
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
    /// 当前 dsh 页 URL(boot 就绪时记录;#3 §7:升级卡片「稍后/返回」的导航目标)。
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

    /// 记录当前 dsh 页 URL(boot / 升级链就绪时调用;「稍后/返回」导航目标,#3 §7)。
    pub(crate) fn record_dsh_url(&self, url: String) {
        if let Ok(mut g) = self.dsh_url.lock() {
            *g = Some(url);
        }
    }

    /// 当前 boot 流水线阶段(升级链确认守卫用:boot 未就绪不升级,#3 §2)。
    pub(crate) fn phase(&self) -> Phase {
        self.state.lock().map(|s| s.phase).unwrap_or(Phase::Error)
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
        }
    }

    /// 阶段迁移:更新状态并推送 `boot-state` 事件(阶段 + 耗时,不含日志)。
    /// emit_to("main"):只投递主窗口 webview,不广播给其它窗口。
    /// 事件同时带 elapsed_secs(从流水线启动起累计)——前端据此显示全程耗时。
    fn set_phase(&self, phase: Phase, error: Option<BootError>) {
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
    fn start_install_progress(&self) -> ProgressTicker {
        let m = self.clone();
        ProgressTicker::start(move |stage, pct| m.emit_install_progress(stage, pct))
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

    fn set_error(&self, error: BootError) {
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

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<Output, ChildWaitError> {
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
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
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

/// 当前 dsh 页 URL(None = 尚未就绪过)。升级卡片「稍后/返回」的导航目标。
pub fn dsh_url(manager: &DshManager) -> Option<String> {
    manager
        .dsh_url
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

/// 导航主窗口到 URL(显示 + 聚焦 + 取消最小化),返回是否成功。
/// boot 就绪导航 / 升级链就绪导航 / 「稍后/返回」导航共用(单一事实来源);
/// update.rs 的 navigate_webview 亦委托本函数。
pub(crate) fn navigate_main_window(app: &tauri::AppHandle, url: &str) -> bool {
    let Some(win) = app.get_webview_window("main") else {
        return false;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    match tauri::Url::parse(url) {
        Ok(u) => match win.navigate(u) {
            Ok(()) => true,
            Err(e) => {
                log::error!("[dsh] 导航失败 {url}: {e}");
                false
            }
        },
        Err(e) => {
            log::error!("[dsh] URL 解析失败 {url}: {e}");
            false
        }
    }
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

/// dsh 服务是否在运行(升级卡「稍后/返回」的恢复判断):句柄在且进程未退出。
/// try_wait 出错(句柄异常/已被他处收割)按未运行处理。
pub fn dsh_is_running(manager: &DshManager) -> bool {
    let Ok(mut guard) = manager.child.lock() else {
        return false;
    };
    let Some(child) = guard.as_mut() else {
        return false;
    };
    matches!(child.try_wait(), Ok(None))
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
/// 失败返回结构化 BootError(版本数据),文案模板在 locale JSON。
fn check_node_version(ver: &str) -> Result<(), BootError> {
    let v = ver.trim().trim_start_matches('v');
    let mut parts = v.split('.');
    let major: u32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(BootError::NodeVersionParseFailed {
            version: ver.to_string(),
        })?;
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ok = (major == 22 && minor >= 19) || major >= 24;
    if !ok {
        return Err(BootError::NodeVersionUnmet {
            current: ver.to_string(),
            required: NODE_REQ.to_string(),
        });
    }
    Ok(())
}

/// 检查 node 是否可用且满足版本要求。`node --version` 带超时:
/// node 被 shim/杀软/网络盘挂起时同步 output() 会永久阻塞,超时后杀进程并报可读错误。
fn check_node() -> Result<String, BootError> {
    let mut binding = Command::new("node");
    let mut child = no_window(&mut binding)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| BootError::NodeMissing {
            required: NODE_REQ.to_string(),
        })?;
    let out = match wait_with_timeout(&mut child, CHECK_NODE_TIMEOUT) {
        Ok(out) => out,
        Err(ChildWaitError::Timeout(_)) => {
            kill_pid_tree(child.id());
            let _ = child.wait();
            return Err(BootError::NodeCheckTimeout {
                seconds: CHECK_NODE_TIMEOUT.as_secs(),
            });
        }
        Err(ChildWaitError::Io(e)) => {
            return Err(BootError::NodeCheckFailed { detail: e });
        }
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!("({stderr})")
        };
        return Err(BootError::NodeVersionCheckFailed {
            exit_code: out.status.code().unwrap_or(-1),
            detail,
        });
    }
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    check_node_version(&ver)?;
    Ok(ver)
}

/// 全局 node_modules 路径,运行时动态解析(`npm root -g`)。
/// 不可写死 %APPDATA%\npm:nvm 等环境 prefix 不同(本机实测 nvm 下为 E:\Nvm\nodejs)。
/// 带超时:`npm root -g` 也会拉起 node,npm/node 被挂起时不得让 boot 卡死在检查阶段。
const NPM_ROOT_TIMEOUT: Duration = Duration::from_secs(10);

fn global_node_modules() -> Result<PathBuf, BootError> {
    let mut cmd = Command::new(if cfg!(windows) { "cmd.exe" } else { "npm" });
    if cfg!(windows) {
        cmd.args(["/c", "npm.cmd"]);
    }
    no_window(&mut cmd)
        .args(["root", "-g"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|_| BootError::NpmRootSpawnFailed)?;
    let out = match wait_with_timeout(&mut child, NPM_ROOT_TIMEOUT) {
        Ok(out) => out,
        Err(ChildWaitError::Timeout(_)) => {
            kill_pid_tree(child.id());
            let _ = child.wait();
            return Err(BootError::NpmRootTimeout {
                seconds: NPM_ROOT_TIMEOUT.as_secs(),
            });
        }
        Err(ChildWaitError::Io(e)) => return Err(BootError::NpmRootIoFailed { detail: e }),
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            String::new()
        } else {
            format!("({stderr})")
        };
        return Err(BootError::NpmRootExitFailed {
            exit_code: out.status.code().unwrap_or(-1),
            detail,
        });
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return Err(BootError::NpmRootEmpty);
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

/// 全局 dsh bin.js 路径(含完整性校验;升级链启动/恢复服务复用)。
pub(crate) fn global_dsh_bin() -> Option<PathBuf> {
    dsh_bin_path(&global_node_modules().ok()?)
}

/// 全局 dsh 已装版本:读 `{prefix}/node_modules/@deepseek-ai/dsh/package.json` 的
/// version 字段(#2 调研:比 npm ls -g 更轻,不受全局树损坏影响)。
/// 未安装 / 读取或解析失败 → None(检测按「无当前版本」处理,不报错)。
pub fn global_dsh_version() -> Option<String> {
    let pkg = global_node_modules()
        .ok()?
        .join("@deepseek-ai/dsh/package.json");
    let text = std::fs::read_to_string(pkg).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("version").and_then(|v| v.as_str()).map(String::from)
}

/// 安装包内置离线缓存目录(若存在)。
/// 约定:<资源目录>/npm-cache,内含 npm cacache。缓存存在性以 cacache 内部
/// 结构为标记:`_cacache/index-v5` 与 `_cacache/content-v2` 两个目录都在才算。
/// 刻意用 cacache 内部结构而非 npm 顶层 `index-v5` 元数据索引:npm 10.9+ 起
/// 不再写顶层 index-v5(元数据并入 _cacache),而 `_cacache` 布局在 npm 7-12
/// 全版本稳定(2026-08-14 实测 npm 10.9.7 生成的缓存;旧标记会把 #6 打包的
/// 缓存漏判为不存在,离线安装静默失效)。
/// 空目录 / 打包遗漏时不满足标记 → 不算缓存,回退网络安装。
fn bundle_cache_dir(resource_dir: &Path) -> Option<PathBuf> {
    let dir = resource_dir.join(BUNDLE_CACHE_REL);
    if dir.join("_cacache/index-v5").is_dir() && dir.join("_cacache/content-v2").is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// 按真实流逝时间给出安装模拟进度(纯函数,可测试)。
///
/// 分段锚点(实测 #2:暖缓存 ~26s、冷缓存 ~4m16s、离线缓存命中秒级):
/// - 0-10s 下载(0% → 60%)——网络为主,占大头
/// - 10-60s 解包写入(60% → 85%)
/// - 60-120s 收尾(85% → 99%),之后停在 99%
///
/// 连续(拐点处百分比相等)、单调不减、**永不提前到 100%**——100% 只能由 npm
/// 进程退出校准(锚点语义,见 boot_pipeline 的 installing 分支)。模拟只做视觉
/// 呈现,不参与任何业务决策(成功/失败/超时全由真实进程事件驱动)。
/// boot 安装与 dsh 升级链(未落地)复用同一逻辑。
pub(crate) fn install_progress_at(elapsed_secs: f64) -> (InstallStage, u8) {
    let t = elapsed_secs.max(0.0);
    // 分段区间内线性插值:拐点处百分比相等,推进平滑无跳变
    let (stage, pct) = if t < 10.0 {
        (InstallStage::Fetching, t / 10.0 * 60.0) // 0-10s:0% → 60%
    } else if t < 60.0 {
        (InstallStage::Reifying, 60.0 + (t - 10.0) * 0.5) // 10-60s:60% → 85%
    } else {
        // 60-120s:85% → 99%;之后封顶 99%
        (InstallStage::Finishing, 85.0 + (t - 60.0).min(60.0) * (14.0 / 60.0))
    };
    (stage, pct.round().clamp(0.0, 99.0) as u8)
}

/// 安装模拟进度线程(boot 安装与 dsh 升级链共用,#7/#17 单一事实来源):
/// 每 500ms 按真实流逝时间推进一次(install_progress_at),百分比变化才回调
/// (安装期事件量 ≈ 200 发/分钟,量级与阶段事件一致);事件去向由调用方决定
/// (boot → boot-state,升级 → upgrade-state)。
///
/// 生命周期契约:调用方必须先 stop_and_join() 再发终态事件(100% 校准 / 错误)
/// ——保证线程在途事件先于终态事件送达,事件流确定性收尾(否则旧进度事件可能
/// 晚于错误事件到达,把前端从错误页拉回 installing 卡死)。stop 置位后线程
/// 最多 500ms 内退出,join 等待不可感知。
///
/// 注意:stop 句柄必须由本结构持有、随实例走——早期实现把 stop 造在线程内部
/// 导致无人能置位,安装路径下 join() 永久挂起(boot 的「全局无 dsh 时安装」
/// 路径从未实机跑过而未暴露,本次升级链落地排查发现并修复)。
pub(crate) struct ProgressTicker {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl ProgressTicker {
    /// 启动进度线程:时间驱动推进(install_progress_at),百分比变化才回调。
    pub(crate) fn start<F>(on_progress: F) -> Self
    where
        F: Fn(InstallStage, u8) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut last_pct: Option<u8> = None;
            while !s.load(Ordering::SeqCst) {
                let (stage, pct) = install_progress_at(started.elapsed().as_secs_f64());
                if last_pct != Some(pct) {
                    last_pct = Some(pct);
                    on_progress(stage, pct);
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
        Self { stop, handle }
    }

    /// 停表并 join(调用方发终态事件前必须调用,见结构文档)。
    pub(crate) fn stop_and_join(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.handle.join();
    }
}

/// npm 全局安装参数组装(纯函数,可测试)。
/// `version_spec`:目标版本规格(boot 传 "@latest",升级链传 "@<pin>",#3 §7)。
/// 离线缓存目录存在时加 `--prefer-offline --cache <目录>`:
/// 缓存命中走本地秒级完成,缺失自动回退网络(用户拍板语义)。
fn npm_install_args(offline_cache: Option<&Path>, version_spec: &str) -> Vec<String> {
    let mut args = vec!["install".to_string(), "-g".to_string()];
    if let Some(dir) = offline_cache {
        args.push("--prefer-offline".into());
        args.push("--cache".into());
        args.push(dir.to_string_lossy().into_owned());
    }
    args.push(format!("@deepseek-ai/dsh{version_spec}"));
    args.extend([
        "--no-audit".into(),
        "--no-fund".into(),
        "--no-progress".into(),
    ]);
    args
}

/// 安装失败的结构化错误(纯函数,可测试)。
/// stderr_tail 是安装期间 stderr 的最后几行:EPERM/EACCES/权限类错误给出
/// 可操作引导(管理员重试 / nvm 用户目录安装 / 手动命令),其余给网络引导。
/// 引导措辞是文案模板,归 locale JSON(errors.InstallFailedPermission/Network);
/// 本函数只产出结构化判别(kind 区分 权限×有无退出码),数据带退出码与 stderr 原文。
/// 结论写进正文:调研实测(#2)确认 npm 失败会保留旧版,失败重试即自愈。
fn install_failure_error(exit_code: Option<i32>, stderr_tail: &[String]) -> BootError {
    // 权限判定用原始行(截断可能切掉行尾的权限标记);文案引导随后按 kind 进 locale JSON
    let is_permission = stderr_tail_has_permission(stderr_tail);
    let stderr_tail = format_stderr_tail(stderr_tail);
    match (is_permission, exit_code) {
        (true, Some(code)) => BootError::InstallFailedPermission {
            exit_code: code,
            stderr_tail,
        },
        (true, None) => BootError::InstallFailedPermissionAbnormal { stderr_tail },
        (false, Some(code)) => BootError::InstallFailedNetwork {
            exit_code: code,
            stderr_tail,
        },
        (false, None) => BootError::InstallFailedNetworkAbnormal { stderr_tail },
    }
}

/// 从 stderr 尾部提取最多 2 行、每行截断到 120 字符,用 "; " 连接;
/// 非空时以 "; " 收尾作模板间分隔符。纯函数,可测试。
fn format_stderr_tail(stderr_tail: &[String]) -> String {
    let mut detail = String::new();
    for l in stderr_tail
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(2)
    {
        if l.chars().count() > 120 {
            detail.push_str(&l.chars().take(120).collect::<String>());
            detail.push('…');
        } else {
            detail.push_str(l);
        }
        detail.push_str("; ");
    }
    detail
}

/// 判定 stderr 尾部是否权限类错误(EPERM/EACCES/…)。纯函数,可测试。
fn stderr_tail_has_permission(stderr_tail: &[String]) -> bool {
    stderr_tail.iter().any(|l| {
        let l = l.to_ascii_lowercase();
        l.contains("eperm")
            || l.contains("eacces")
            || l.contains("permission denied")
            || l.contains("lack permission")
    })
}

/// npm 全局安装 dsh(boot 传 "@latest" 跟随 latest;dsh 升级链传 "@<pin>" 精确版本,#3 §7),
/// stdout/stderr 逐行转发为日志事件。install_pid 登记:升级期间用户退出时随退出收敛一并杀。
/// Windows 上 CreateProcess 不能直接执行 .cmd/.bat(npm 是 .cmd shim),须经 cmd.exe /c 包装。
/// 带 NPM_INSTALL_TIMEOUT 超时;超时按进程树杀并报可读错误。
/// 安装包内置离线缓存存在时优先走离线安装(见 bundle_cache_dir)。
pub(crate) fn npm_install_global(manager: &DshManager, version_spec: &str) -> Result<(), BootError> {
    let cache_dir = manager
        .app
        .path()
        .resource_dir()
        .ok()
        .as_deref()
        .and_then(bundle_cache_dir);
    let args = npm_install_args(cache_dir.as_deref(), version_spec);
    if let Some(dir) = &cache_dir {
        log::info!("[dsh] 使用安装包内置离线缓存: {}", dir.display());
    }

    let mut cmd = Command::new(if cfg!(windows) { "cmd.exe" } else { "npm" });
    if cfg!(windows) {
        cmd.args(["/c", "npm.cmd"]);
    }
    no_window(&mut cmd)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| BootError::NpmSpawnFailed { detail: e.to_string() })?;
    // 登记安装中进程,退出收敛时一并杀(quit_app/托盘退出)
    manager.set_install_pid(child.id());

    let stdout = child.stdout.take().expect("piped stdout");
    let m = manager.clone();
    let out_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            m.push_log("stdout", line);
        }
    });

    let stderr = child.stderr.take().expect("piped stderr");
    let m = manager.clone();
    // stderr 尾部捕获:失败时拼进可读错误信息(EPERM 权限引导等),与日志流并行
    let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tail = stderr_tail.clone();
    let err_thread = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            m.push_log("stderr", line.clone());
            let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
            t.push(line);
            while t.len() > 8 {
                t.remove(0);
            }
        }
    });

    let result = wait_with_timeout(&mut child, NPM_INSTALL_TIMEOUT);
    manager.clear_install_pid();
    match result {
        Ok(out) => {
            // 进程已退出 → 管道 EOF → 读线程自然结束,join 确保 stderr 尾部收全
            let _ = out_thread.join();
            let _ = err_thread.join();
            let tail: Vec<String> = stderr_tail
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            if out.status.success() {
                Ok(())
            } else {
                Err(install_failure_error(out.status.code(), &tail))
            }
        }
        Err(ChildWaitError::Timeout(_)) => {
            kill_pid_tree(child.id());
            let _ = child.wait();
            // 杀进程后管道 EOF,join 防读线程泄漏
            let _ = out_thread.join();
            let _ = err_thread.join();
            Err(BootError::InstallTimeout {
                seconds: NPM_INSTALL_TIMEOUT.as_secs(),
            })
        }
        Err(ChildWaitError::Io(e)) => Err(BootError::NpmInstallIoFailed { detail: e }),
    }
}

/// 启动 `node <bin.js> web --port 0`,返回 stdout/stderr 合流后的行接收端
/// (boot 与升级链复用;升级链用同一 bin 路径,升级前后不变,#2 调研)
pub(crate) fn spawn_dsh(
    manager: &DshManager,
    bin: &Path,
) -> Result<Receiver<(String, String)>, BootError> {
    let mut binding = Command::new("node");
    let cmd = no_window(&mut binding)
        .arg(bin)
        .args(["web", "--port", "0"])
        .env("DSH_TELEMETRY_DISABLED", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| BootError::DshSpawnFailed { detail: e.to_string() })?;

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
) -> Result<(u16, Receiver<(String, String)>), BootError> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok((stream, line)) => {
                manager.push_log(&stream, line.clone());
                if let Some(port) = parse_ready_line(&line) {
                    if tcp_wait(port) {
                        return Ok((port, rx));
                    }
                    return Err(BootError::ReadyPortUnavailable { port });
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // 子进程是否已退出(锁作用域内完成判断,避免守卫借用逃逸)
                if let Some(code) = child_exit_code(manager) {
                    return Err(BootError::DshExitedEarly { exit_code: code });
                }
                if Instant::now() >= deadline {
                    return Err(BootError::DshStartTimeout {
                        seconds: START_TIMEOUT.as_secs(),
                    });
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // 输出流关闭 = 读线程结束 = 子进程已退出(或句柄被继承导致异常断流)
                return match child_exit_code(manager) {
                    Some(code) => Err(BootError::DshExitedEarly { exit_code: code }),
                    None => Err(BootError::DshExitedEarlyNoCode),
                };
            }
        }
    }
}

// ── boot 流水线(运行在工作线程上)─────────────────────────────────

fn boot_pipeline(manager: &DshManager) {
    // 1. 环境检查
    log::info!("[dsh] boot: checking node…");
    manager.set_phase(Phase::Checking, None);
    let node_version = match check_node() {
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
    let bin = match global_dsh_bin() {
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
            match global_dsh_bin() {
                Some(b) => b,
                None => {
                    manager.set_error(BootError::InstallVerifyFailed);
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

    // 6. 就绪:记录 dsh URL(升级卡片「稍后/返回」导航目标,#3 §7)→ 导航窗口
    //    到 dsh Web UI,窗口自此变纯 dsh 页面(只做显示,不干扰功能)
    manager.set_phase(Phase::Ready, None);
    let url = format!("http://127.0.0.1:{port}");
    manager.record_dsh_url(url.clone());
    log::info!("[dsh] boot: navigate → {url}");
    if !navigate_main_window(&manager.app, &url) {
        // 导航失败:dsh 仍在运行,先杀子进程再进错误态(否则重试会再起一个 dsh)
        kill_child(manager);
        manager.set_error(BootError::NavigateFailed);
        return;
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
            let ok = status.map(|s| s.success()).unwrap_or(false);
            log::info!(
                "[dsh] reaper: dsh 子进程退出, ok={ok}, quitting={}, upgrade_active={}",
                is_quitting(),
                upgrade_active()
            );
            // 升级链主动杀旧 dsh 也是非零退出:UPGRADE_ACTIVE 抑制误报弹窗
            // (独立于 is_quitting——升级不退出应用,关闭三选对话框保持有效,#3 §2)
            if !ok && !is_quitting() && !upgrade_active() {
                // 原生对话框文案跟随系统语言(与托盘/关闭对话框同源,见 locales.rs)
                let texts = crate::locales::shell_texts(crate::locales::detect_lang());
                let _ = manager
                    .app
                    .dialog()
                    .message(texts.dsh_crashed)
                    .title("DeepSeek Desktop")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }
        }
    });
}

// ── Tauri commands ─────────────────────────────────────────────────

/// 触发(或重试)boot 流水线,并返回含最近日志的状态快照。
/// 挂载/重试一调两用:触发 + 拉快照,命令面从 3 收窄到 2(boot/quit_app)。
/// phase 守卫:非 Idle/Error 时 no-op(防 StrictMode 双 invoke),快照仍正常返回。
/// async + Result:不占用 IPC/主线程(tauri 2 要求含引用输入的 async command 返回 Result)。
#[tauri::command]
pub async fn boot(state: tauri::State<'_, DshManager>) -> Result<BootStateSnapshot, String> {
    boot_start(state.inner());
    Ok(state.inner().snapshot())
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
            m.set_error(BootError::Internal { message: msg });
        }
    });
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
    fn check_node_version_accepts_required_ranges() {
        assert!(check_node_version("v22.22.2").is_ok());
        assert!(check_node_version("v22.19.0").is_ok());
        assert!(check_node_version("v24.0.0").is_ok());
        assert!(check_node_version("v25.1.0").is_ok());
        assert!(check_node_version("22.22.2").is_ok()); // 无 v 前缀
    }

    #[test]
    fn check_node_version_rejects_others() {
        // 22 系但 < 19 → 版本不满足(带当前版本与要求,供 locale 模板插值)
        assert!(matches!(
            check_node_version("v22.18.0"),
            Err(BootError::NodeVersionUnmet { current, required })
                if current == "v22.18.0" && required == NODE_REQ
        ));
        // 23 不在 ^22.19 || >=24 内
        assert!(matches!(
            check_node_version("v23.0.0"),
            Err(BootError::NodeVersionUnmet { .. })
        ));
        assert!(matches!(
            check_node_version("v20.10.0"),
            Err(BootError::NodeVersionUnmet { .. })
        ));
        // 不可解析的版本号 → 解析失败
        assert!(matches!(
            check_node_version("not-a-version"),
            Err(BootError::NodeVersionParseFailed { .. })
        ));
    }

    #[test]
    fn check_node_version_handles_short_forms_and_whitespace() {
        // 缺段容错(minor 缺省按 0):^22.19 边界在 minor 上,22.x 缺 minor 视为 22.0
        assert!(matches!(
            check_node_version("v22"),
            Err(BootError::NodeVersionUnmet { .. })
        ));
        assert!(check_node_version("v22.19").is_ok()); // 22.19 无 patch
        assert!(check_node_version("24").is_ok()); // >=24 无 minor
        assert!(check_node_version("v24").is_ok());
        // 首尾空白(检查输出 / 管道可能带换行,调用方先 trim 再进比较,单测守住边界)
        assert!(check_node_version("  v22.22.2  ").is_ok());
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
        let mut child = Command::new("cmd")
            .args(["/c", "echo ok"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
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
        assert!(
            matches!(r, Err(ChildWaitError::Timeout(_))),
            "期望 Timeout,实际 {r:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2), "超时检测过慢");
        kill_pid_tree(child.id());
        let _ = child.wait();
    }

    #[test]
    fn dsh_bin_path_requires_bin_js_not_just_version() {
        // 半残安装(版本号在、bin.js 缺)不得视为「已安装」——否则被「有则用」跳过,
        // 坏掉的安装永远修不好(2026-08-14 实测事故)
        let dir = std::env::temp_dir().join(format!("dsh-boot-test-{}", std::process::id()));
        let dsh_dir = dir.join("@deepseek-ai/dsh");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dsh_dir.join("lib")).unwrap();

        // 只有 package.json(无 bin.js)→ 视为未安装
        std::fs::write(dsh_dir.join("package.json"), r#"{"version":"0.1.0-rc.6"}"#).unwrap();
        assert_eq!(
            dsh_bin_path(&dir),
            None,
            "版本号存在但 bin.js 缺失必须判为未安装"
        );

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
        assert!(
            out.status.success(),
            "npm.cmd 经 cmd.exe 启动失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }

    #[test]
    fn npm_install_args_without_bundle_cache_uses_plain_latest() {
        // 无离线缓存:普通网络安装(生产路径参数必须逐字一致)
        assert_eq!(
            npm_install_args(None, "@latest"),
            vec![
                "install",
                "-g",
                "@deepseek-ai/dsh@latest",
                "--no-audit",
                "--no-fund",
                "--no-progress"
            ]
        );
    }

    #[test]
    fn npm_install_args_with_bundle_cache_prefers_offline() {
        // 有离线缓存:加 --prefer-offline --cache <目录>(缓存命中秒级、缺失回退网络)
        let cache = Path::new("C:/app resources/npm-cache");
        assert_eq!(
            npm_install_args(Some(cache), "@latest"),
            vec![
                "install",
                "-g",
                "--prefer-offline",
                "--cache",
                "C:/app resources/npm-cache",
                "@deepseek-ai/dsh@latest",
                "--no-audit",
                "--no-fund",
                "--no-progress"
            ]
        );
    }

    #[test]
    fn npm_install_args_pins_version_for_upgrade() {
        // 升级链传精确 pin(#3 §7):`npm install -g @deepseek-ai/dsh@<pin>`,
        // 不裸用 @latest;其余参数与 boot 完全一致(同一 npm 机制)
        assert_eq!(
            npm_install_args(None, "@0.1.0-rc.6"),
            vec![
                "install",
                "-g",
                "@deepseek-ai/dsh@0.1.0-rc.6",
                "--no-audit",
                "--no-fund",
                "--no-progress"
            ]
        );
    }

    #[test]
    fn bundle_cache_dir_detects_real_cacache() {
        // 生产路径:resource_dir() 下 <npm-cache> 目录;空目录不得误判为离线缓存
        let dir = std::env::temp_dir().join(format!("dsh-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // 目录不存在 → None(开发态资源目录里没有 npm-cache,走网络安装)
        assert_eq!(bundle_cache_dir(&dir), None);
        // 空目录不算缓存(打包遗漏时不得误判离线)
        std::fs::create_dir_all(dir.join("npm-cache")).unwrap();
        assert_eq!(bundle_cache_dir(&dir), None);
        // 只有 npm 顶层 index-v5(旧 npm ≤10.8 元数据索引)也不算 ——
        // 标记看 cacache 内部结构,不是 npm 顶层目录
        std::fs::create_dir_all(dir.join("npm-cache/index-v5")).unwrap();
        assert_eq!(bundle_cache_dir(&dir), None);
        // 带 cacache 内部标记(_cacache/index-v5 + _cacache/content-v2)→ 视为存在
        std::fs::create_dir_all(dir.join("npm-cache/_cacache/index-v5")).unwrap();
        std::fs::create_dir_all(dir.join("npm-cache/_cacache/content-v2")).unwrap();
        assert_eq!(bundle_cache_dir(&dir), Some(dir.join("npm-cache")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_failure_error_guides_on_permission_error() {
        // 权限失败(npm 实测报错形态:EPERM + "you lack permissions to access it")
        let tail = vec![
            "npm error code EPERM".to_string(),
            "npm error The operation was rejected by your operating system.".to_string(),
            "npm error you lack permissions to access it".to_string(),
        ];
        assert_eq!(
            install_failure_error(Some(243), &tail),
            BootError::InstallFailedPermission {
                exit_code: 243,
                stderr_tail:
                    "npm error code EPERM; npm error The operation was rejected by your operating system.; "
                        .to_string(),
            }
        );

        // EACCES 同理,无退出码 → Abnormal 变体
        assert_eq!(
            install_failure_error(None, &["npm error EACCES: permission denied".into()]),
            BootError::InstallFailedPermissionAbnormal {
                stderr_tail: "npm error EACCES: permission denied; ".to_string(),
            }
        );
    }

    #[test]
    fn install_failure_error_falls_back_to_network_variant() {
        // 非权限类失败 → Network 变体(网络引导文案模板在 locale JSON)
        assert_eq!(
            install_failure_error(Some(1), &["npm error code ENOTFOUND".into()]),
            BootError::InstallFailedNetwork {
                exit_code: 1,
                stderr_tail: "npm error code ENOTFOUND; ".to_string(),
            }
        );

        // 无 stderr 输出:stderr_tail 为空串,模板可干净衔接引导语
        assert_eq!(
            install_failure_error(Some(1), &[]),
            BootError::InstallFailedNetwork {
                exit_code: 1,
                stderr_tail: String::new(),
            }
        );
    }

    #[test]
    fn format_stderr_tail_truncates_long_lines() {
        // 生产路径:每行截断到 120 字符 + 省略号,最多 2 行,'; ' 连接
        let out = format_stderr_tail(&["x".repeat(200), "second".to_string()]);
        assert!(out.starts_with(&"x".repeat(120)), "{out}");
        assert!(out.contains('…'));
        assert!(out.ends_with("second; "));
    }

    #[test]
    fn format_stderr_tail_skips_blank_lines() {
        assert_eq!(format_stderr_tail(&[]), "");
        assert_eq!(format_stderr_tail(&["   ".to_string(), "ok".to_string()]), "ok; ");
    }

    #[test]
    fn boot_error_serializes_as_kind_and_data() {
        // 前端 toStructuredError 依赖的线上契约:tag/content 判别式,
        // 字段 camelCase;unit 变体无 data 字段
        assert_eq!(
            serde_json::to_value(BootError::NodeCheckTimeout { seconds: 10 }).unwrap(),
            serde_json::json!({ "kind": "NodeCheckTimeout", "data": { "seconds": 10 } })
        );
        assert_eq!(
            serde_json::to_value(BootError::DshExitedEarly { exit_code: 1 }).unwrap(),
            serde_json::json!({ "kind": "DshExitedEarly", "data": { "exitCode": 1 } })
        );
        assert_eq!(
            serde_json::to_value(BootError::NodeMissing {
                required: NODE_REQ.to_string()
            })
            .unwrap(),
            serde_json::json!({ "kind": "NodeMissing", "data": { "required": "Node.js ^22.19 or >=24" } })
        );
    }

    #[cfg(windows)]
    #[test]
    fn npm_cache_arg_with_spaces_survives_cmd_exe() {
        // 生产路径:离线安装要把含空格的 --cache 路径经 cmd.exe /c 原样传给 npm.cmd。
        // npm config get 不回源网络,验证 Rust 自动加引号的参数不被 cmd 拆坏。
        let cache = "C:/spaced cache dir/npm-cache";
        let out = Command::new("cmd.exe")
            .args(["/c", "npm.cmd", "config", "get", "cache", "--cache", cache])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "npm 执行失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("spaced cache dir"),
            "含空格 cache 路径被 cmd 拆坏: {stdout}"
        );
    }

    // ── 安装进度模拟(#7)──────────────────────────────────────────

    #[test]
    fn install_progress_anchors_match_design() {
        // 生产路径:boot_pipeline 的进度线程每 500ms 调 install_progress_at 一次,
        // 阶段与百分比驱动前端进度条与阶段文案。锚点必须与设计一致。
        assert_eq!(install_progress_at(0.0), (InstallStage::Fetching, 0));
        assert_eq!(install_progress_at(5.0), (InstallStage::Fetching, 30));
        assert_eq!(install_progress_at(10.0), (InstallStage::Reifying, 60)); // 拐点连续
        assert_eq!(install_progress_at(35.0), (InstallStage::Reifying, 73)); // 60 + 25*0.5
        assert_eq!(install_progress_at(60.0), (InstallStage::Finishing, 85)); // 拐点连续
        assert_eq!(install_progress_at(120.0), (InstallStage::Finishing, 99));
        assert_eq!(install_progress_at(600.0), (InstallStage::Finishing, 99)); // 封顶
    }

    #[test]
    fn install_progress_is_monotonic_and_never_reaches_100() {
        // 模拟进度单调不减、永不提前 100%(100% 只能由 npm 进程退出校准):
        // 失败/超时路径不会出现「已 100% 却失败」的矛盾呈现
        let mut prev = 0u8;
        for secs in (0..300).map(|s| s as f64 + 0.1) {
            let (_, pct) = install_progress_at(secs);
            assert!(pct >= prev, "进度回退: {secs}s → {pct} < {prev}");
            assert!(pct < 100, "模拟进度提前到 100%: {secs}s");
            prev = pct;
        }
    }

    #[test]
    fn install_progress_offline_cache_fast_path_stays_low() {
        // 离线缓存命中秒级完成(#16):安装开始即校准 100%,模拟值短暂出现且远低于
        // 100%——快路径下进度条从 0 快速跳到 100 是预期行为,不得到处乱跳
        let (stage, pct) = install_progress_at(1.5);
        assert_eq!(stage, InstallStage::Fetching);
        assert!(pct < 20, "离线快路径下 1.5s 进度应很低,实际 {pct}");
        let (_, pct) = install_progress_at(3.0);
        assert!(pct < 30, "离线快路径下 3s 进度应较低,实际 {pct}");
    }

    #[test]
    fn install_progress_clamps_negative_input() {
        // 防御:负流逝时间(时钟异常)按 0 处理
        assert_eq!(install_progress_at(-1.0), (InstallStage::Fetching, 0));
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
