//! dsh 升级链(#17 落地,#3 设计定稿):检测 + 提示 + 独立升级流水线。
//!
//! 架构(与 update.rs 同款):检查/安装/重启全部在 Rust 侧(本外壳的 dsh 页是
//! remote origin,收不到任何 Tauri 事件/命令,#9),前端只是本地页上的镜像视图
//! (挂载时拉 `upgrade_state` 快照 + 监听 `upgrade-state` 事件)。
//!
//! 检测(#1/#3 定稿,与应用升级共用触发时机):启动探测 + 6h 轮询(常量复用
//! update::POLL_INTERVAL)+ 托盘「检查更新」手动入口(组合编排在 tray.rs)。
//! 方式 = registry API 直查 abbreviated packument(install-v1,23KB),3-5s
//! 短超时,任何异常一律静默按无新版(#2 调研);比较 dist-tags.latest 与全局
//! 已装版本(读 package.json version),latest > 已装 才提示(降级不提示,
//! 见 latest_is_newer 注释)。
//!
//! 提示(#3 §1):自动检测发现新版 → 托盘徽标变体 + 动态菜单项「升级 dsh 到 vX」
//! + tooltip,不弹窗打断;手动检查 → 原生对话框直接回答([升级] → 导航升级卡片
//! 并自动开始流水线,#3:确认即授权,不二次确认)。
//!
//! 流水线(#3 §2 定稿,独立于 boot 状态机;底层工具全部复用 dsh.rs,
//! 禁止复制实现):
//!   confirm → killing(杀当前 dsh,UPGRADE_ACTIVE 抑制 reaper 误报)
//!          → installing(`npm install -g @deepseek-ai/dsh@<pin>`,复用
//!            npm_install_global 与 install_pid 治理;进度模拟复用
//!            install_progress_at / ProgressTicker,锚点 = 进程退出 + 校准 100%)
//!          → verify(读全局 package.json version == pin 且 bin.js 完整)
//!          → starting(spawn_dsh + wait_ready,新版 bin 路径不变)
//!          → ready(导航窗口回 dsh 页,新端口 URL)
//!
//! 失败处理(#3 §3):失败保留旧版(npm 语义,#2 实测)+ 恢复服务——
//! [返回 dsh] 经 upgrade_dismiss:Rust 侧检查 dsh 是否在运行,未运行则起当前
//! 全局安装(失败时旧版保留/新版已装好,都是「当前全局版本」)→ 就绪 → 导航,
//! 不能「只关卡片」。错误结构化:升级特有 kind(UpgradeKillFailed /
//! UpgradeVerifyFailed)独立枚举,安装/启动类直接以 BootError 形态传播,
//! 前端统一按 kind 翻译(errors.<kind> 键,零额外机制)。

use std::cmp::Ordering;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult};

use crate::{dsh, locales, tray, update};

/// registry abbreviated packument(install-v1 Accept 头,23KB,#2 调研实测)。
const REGISTRY_URL: &str = "https://registry.npmjs.org/@deepseek-ai/dsh";
/// 直查超时(#2 定稿 3-5s):超时/任何异常静默按无新版,不影响启动。
const CHECK_TIMEOUT: Duration = Duration::from_secs(4);

// ── 全局守卫(跨线程)────────────────────────────────────────────────

/// 流水线防重入(与 dsh::BOOTING 同款:状态守卫 + 原子双保险——两个并发
/// confirm 都可能先读到 Available,原子位挡住第二个)。
static PIPELINE_RUNNING: AtomicBool = AtomicBool::new(false);

// ── 纯函数:registry 解析 / 版本比较 ─────────────────────────────────

/// registry abbreviated packument 解析:提取 dist-tags.latest
/// (比较对象是 latest 标签,不是最高版本号,#2 调研)。纯函数,可测。
fn parse_dist_latest(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let latest = v.get("dist-tags")?.get("latest")?;
    latest.as_str().map(String::from)
}

/// 版本归一化(去首尾空白 / 前导 v),比较与校验共用。
fn normalize_version(s: &str) -> &str {
    s.trim().trim_start_matches('v')
}

/// 版本相等(升级后校验用):归一化后逐字比较。pin 与 package.json version
/// 都是 registry 全版本号(如 0.1.0-rc.6),归一化足够,无需 semver 语义。
fn version_eq(a: &str, b: &str) -> bool {
    normalize_version(a) == normalize_version(b)
}

/// prerelease 段(semver 规则):数字段按数值、字母段按字典序,数字段 < 字母段。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleasePart {
    Num(u64),
    Word(String),
}

impl From<&str> for PrereleasePart {
    fn from(s: &str) -> Self {
        if s.chars().all(|c| c.is_ascii_digit()) {
            PrereleasePart::Num(s.parse().unwrap_or(0))
        } else {
            PrereleasePart::Word(s.to_string())
        }
    }
}

fn cmp_prerelease_part(a: &PrereleasePart, b: &PrereleasePart) -> Ordering {
    match (a, b) {
        (PrereleasePart::Num(x), PrereleasePart::Num(y)) => x.cmp(y),
        (PrereleasePart::Word(x), PrereleasePart::Word(y)) => x.cmp(y),
        (PrereleasePart::Num(_), PrereleasePart::Word(_)) => Ordering::Less,
        (PrereleasePart::Word(_), PrereleasePart::Num(_)) => Ordering::Greater,
    }
}

/// 解析 major.minor.patch[-prerelease]。minor/patch 缺省按 0(如 "0.1"、"1");
/// 多余段(1.2.3.4)视为非法。prerelease 为空 = release 版。
fn parse_version(s: &str) -> Option<(u64, u64, u64, Vec<PrereleasePart>)> {
    let s = normalize_version(s);
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (s, None),
    };
    let mut it = core.split('.');
    let major: u64 = it.next()?.parse().ok()?;
    let minor: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let patch: u64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    if it.next().is_some() {
        return None;
    }
    let pre = pre
        .map(|p| p.split('.').map(PrereleasePart::from).collect::<Vec<_>>());
    Some((major, minor, patch, pre.unwrap_or_default()))
}

/// semver 比较(纯函数,可测):major.minor.patch[-prerelease]。
/// prerelease 按 semver 规则逐段比较,有 prerelease < 无 prerelease(release);
/// 解析失败返回 Equal(检测静默按无新版处理,不为脏数据报错)。
fn semver_cmp(a: &str, b: &str) -> Ordering {
    let (Some((ma, na, pa, ra)), Some((mb, nb, pb, rb))) = (parse_version(a), parse_version(b))
    else {
        return Ordering::Equal;
    };
    match (ma.cmp(&mb), na.cmp(&nb), pa.cmp(&pb)) {
        (Ordering::Equal, Ordering::Equal, Ordering::Equal) => cmp_prerelease(&ra, &rb),
        (Ordering::Greater, ..) => Ordering::Greater,
        (Ordering::Less, ..) => Ordering::Less,
        (Ordering::Equal, Ordering::Greater, ..) => Ordering::Greater,
        (Ordering::Equal, Ordering::Less, ..) => Ordering::Less,
        (Ordering::Equal, Ordering::Equal, Ordering::Greater) => Ordering::Greater,
        (Ordering::Equal, Ordering::Equal, Ordering::Less) => Ordering::Less,
    }
}

fn cmp_prerelease(ra: &[PrereleasePart], rb: &[PrereleasePart]) -> Ordering {
    match (ra.is_empty(), rb.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater, // release > prerelease
        (false, true) => Ordering::Less,
        (false, false) => {
            for (x, y) in ra.iter().zip(rb) {
                let o = cmp_prerelease_part(x, y);
                if o != Ordering::Equal {
                    return o;
                }
            }
            ra.len().cmp(&rb.len())
        }
    }
}

/// latest 是否比已装版本新(纯函数,可测)。
/// #3 §5 的判据是「dist-tags.latest != 已装版本」;此处按推荐收窄为 semver
/// `>` ——「≠」含降级场景(用户装过比 latest 更新的开发版),提示降级是
/// 错误行为;npm 的 @latest 语义本就是跟随发布标签,不提示即不打扰。
fn latest_is_newer(latest: &str, current: &str) -> bool {
    semver_cmp(latest, current) == Ordering::Greater
}

// ── 状态与视图 ─────────────────────────────────────────────────────

/// 升级流水线阶段(#3 §2:kill → install → verify → start → ready)。
/// 事件/快照序列化为小写串("killing"/"installing"/"verifying"/"starting")。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum UpgradePhase {
    /// 杀当前 dsh 进程
    Killing,
    /// npm 全局安装 pin 版本(progress/stage 仅本阶段携带)
    Installing,
    /// 版本校验(读全局 package.json version == pin)
    Verifying,
    /// 启动新版 dsh(spawn + wait_ready)
    Starting,
}

/// upgrade-state 事件/命令返回:内部 tag status,字段 camelCase。
/// Active 携带 phase;progress/stage 仅 installing 携带(skip_serializing_if)。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status", rename_all_fields = "camelCase")]
pub enum UpgradeStateView {
    /// 无待升级(初始 / 无新版 / 升级成功消费完毕)
    Idle,
    /// 发现新版,等待用户确认(version = latest,current_version = 已装)
    Available {
        version: String,
        current_version: String,
    },
    /// 升级流水线运行中
    Active {
        phase: UpgradePhase,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stage: Option<dsh::InstallStage>,
    },
    /// 升级成功(瞬态:Rust 随即导航回 dsh 页)
    Ready,
    /// 升级失败(旧版保留 + 恢复服务;#3 §3)。
    /// version = 目标 pin(重试继续用同一版本,install 幂等且缓存命中快)
    Failed { version: String, error: UpgradeError },
}

/// 升级特有错误(kind 与 boot 不重名;#3 §3)。文案模板在 locale JSON
/// 的 `errors.<kind>` 键,数据只携带运行时事实。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "PascalCase", rename_all_fields = "camelCase")]
pub(crate) enum UpgradeErrorKind {
    /// 无法停止当前 dsh 服务(杀后超时仍存活)
    UpgradeKillFailed { detail: String },
    /// 升级后版本校验失败(全局 version ≠ pin 或 bin.js 缺失)
    UpgradeVerifyFailed,
}

/// 升级失败的结构化原因:升级特有错误 + 与 boot 共用的安装/启动类错误直接以
/// BootError 形态透传(复用流水线函数返回类型,前端统一按 errors.<kind> 翻译,
/// 零额外机制,#3 §3)。untagged 序列化为单一 {kind,data} 形态。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum UpgradeError {
    Kind(UpgradeErrorKind),
    Boot(dsh::BootError),
}

/// 状态机事件。由检查/流水线各动作产生,经 `apply_event` 归约。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UpgradeEvent {
    /// 检查发现新版(version = latest,current_version = 已装)
    Found { version: String, current_version: String },
    /// 检查无新版 / 未安装
    NoneFound,
    /// 用户确认,流水线启动(→ Active{Killing};同步归约,防双流水线)
    Started,
    /// 流水线阶段迁移(progress/stage 仅 installing 携带)
    PhaseChanged {
        phase: UpgradePhase,
        progress: Option<u8>,
        stage: Option<dsh::InstallStage>,
    },
    /// 安装进度(模拟推进;100 只能由 npm 进程退出校准)
    Progress { progress: u8, stage: dsh::InstallStage },
    /// 流水线成功(→ Ready)
    Succeeded,
    /// 流水线失败(→ Failed{version, error})
    Failed { version: String, error: UpgradeError },
    /// 升级链消费完毕(导航成功后 → Idle)
    Reset,
}

/// 状态机归约:事件 → 新状态。纯函数,可测;
/// 生产路径唯一调用方是 `UpgradeManager::reduce`(先归约后下发,不跑分叉逻辑)。
pub fn apply_event(state: &UpgradeStateView, event: UpgradeEvent) -> UpgradeStateView {
    match event {
        // 流水线在途时忽略检查结果(检查在 Active 时本就跳过,此处防御)
        UpgradeEvent::Found { version, current_version } => {
            if matches!(state, UpgradeStateView::Active { .. }) {
                state.clone()
            } else {
                UpgradeStateView::Available { version, current_version }
            }
        }
        UpgradeEvent::NoneFound => {
            if matches!(state, UpgradeStateView::Active { .. }) {
                state.clone()
            } else {
                UpgradeStateView::Idle
            }
        }
        UpgradeEvent::Started => match state {
            UpgradeStateView::Available { .. } | UpgradeStateView::Failed { .. } => {
                UpgradeStateView::Active {
                    phase: UpgradePhase::Killing,
                    progress: None,
                    stage: None,
                }
            }
            _ => state.clone(),
        },
        UpgradeEvent::PhaseChanged { phase, progress, stage } => {
            if matches!(state, UpgradeStateView::Active { .. }) {
                UpgradeStateView::Active { phase, progress, stage }
            } else {
                state.clone()
            }
        }
        UpgradeEvent::Progress { progress, stage } => {
            if matches!(state, UpgradeStateView::Active { .. }) {
                UpgradeStateView::Active {
                    phase: UpgradePhase::Installing,
                    progress: Some(progress),
                    stage: Some(stage),
                }
            } else {
                state.clone()
            }
        }
        UpgradeEvent::Succeeded => {
            if matches!(state, UpgradeStateView::Active { .. }) {
                UpgradeStateView::Ready
            } else {
                state.clone()
            }
        }
        UpgradeEvent::Failed { version, error } => {
            if matches!(state, UpgradeStateView::Active { .. }) {
                UpgradeStateView::Failed { version, error }
            } else {
                state.clone()
            }
        }
        UpgradeEvent::Reset => {
            if matches!(state, UpgradeStateView::Ready) {
                UpgradeStateView::Idle
            } else {
                state.clone()
            }
        }
    }
}

// ── 管理器 ─────────────────────────────────────────────────────────

/// dsh 升级管理器。Clone 共享内部状态(检查线程/流水线线程各持一份)。
#[derive(Clone)]
pub struct UpgradeManager {
    app: AppHandle,
    state: Arc<Mutex<UpgradeStateView>>,
}

impl UpgradeManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            state: Arc::new(Mutex::new(UpgradeStateView::Idle)),
        }
    }

    pub fn snapshot(&self) -> UpgradeStateView {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 状态归约 + 下发(生产路径唯一入口,测试的 apply_event 即此处的归约)。
    fn reduce(&self, event: UpgradeEvent) {
        let next = {
            let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let next = apply_event(&s, event);
            *s = next.clone();
            next
        };
        log::info!("[upgrade] state → {next:?}");
        let _ = self.app.emit_to("main", "upgrade-state", next);
    }

    /// 流水线在途(检查与手动入口的 no-op 守卫,#3 边界)。
    pub(crate) fn is_pipeline_running(&self) -> bool {
        matches!(self.snapshot(), UpgradeStateView::Active { .. })
    }

    /// 常驻检查(setup 调用一次):启动探测 + 6h 轮询。
    /// 与应用升级共用触发时机(常量复用 update::POLL_INTERVAL),独立运行。
    pub fn start_resident_checks(&self) {
        self.check_now();
        let m = self.clone();
        thread::spawn(move || loop {
            thread::sleep(update::POLL_INTERVAL);
            m.check_now();
        });
    }

    /// 自动检查(启动 / 6h):静默——发现新版只亮托盘徽标/菜单项,不弹窗(#3 §1)。
    fn check_now(&self) {
        if self.is_pipeline_running() {
            log::info!("[upgrade] 升级流水线在途,跳过自动检查");
            return;
        }
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = run_check(&app).await;
        });
    }

    /// 确认升级(卡片「立即升级/重试」与手动检查对话框[升级]共用入口)。
    /// 守卫(#3 §2):boot 未就绪(phase 非 Ready)拒绝;状态非 Available/Failed
    /// 拒绝;流水线在途拒绝。启动成功后同步归约 Started(前端立即看到 Active)。
    pub(crate) fn confirm_start(&self, dsh: &dsh::DshManager) -> bool {
        if dsh.phase() != dsh::Phase::Ready {
            log::warn!("[upgrade] 确认升级被拒:boot 未就绪(启动未完成不升级)");
            return false;
        }
        let pin = match self.snapshot() {
            UpgradeStateView::Available { version, .. } | UpgradeStateView::Failed { version, .. } => {
                version
            }
            other => {
                log::warn!("[upgrade] 确认升级被拒:状态 {other:?} 不允许");
                return false;
            }
        };
        if PIPELINE_RUNNING.swap(true, AtomicOrdering::SeqCst) {
            log::warn!("[upgrade] 确认升级被拒:流水线已在途");
            return false;
        }
        self.reduce(UpgradeEvent::Started);
        let up = self.clone();
        let dsh = dsh.clone();
        thread::spawn(move || {
            // catch_unwind:流水线线程 panic 会让状态永远停在 Active(卡片永久转圈),
            // 捕获后转为可见错误(panic 本身已由 logging hook 落盘)
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                upgrade_pipeline(&up, &dsh, &pin)
            }));
            PIPELINE_RUNNING.store(false, AtomicOrdering::SeqCst);
            if let Err(panic) = result {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "升级流水线内部错误".into());
                log::error!("[upgrade] 升级流水线 panic: {msg}");
                dsh::set_upgrade_active(false);
                up.reduce(UpgradeEvent::Failed {
                    version: pin,
                    error: UpgradeError::Boot(dsh::BootError::Internal { message: msg }),
                });
            }
        });
        true
    }

    /// 升级卡片「稍后/返回 dsh」(#3 §3):Rust 侧保证 dsh 在跑——未运行则起
    /// 当前全局安装(失败保留旧版 / 新版已装好,都是「当前全局版本」)→ 就绪 →
    /// 导航回 dsh 页。不能「只关卡片」,否则用户回到打不开的页面。
    pub fn dismiss(&self, dsh: &dsh::DshManager) {
        if dsh::dsh_is_running(dsh) {
            if let Some(url) = dsh::dsh_url(dsh) {
                log::info!("[upgrade] 返回 dsh(dsh 仍在运行)→ {url}");
                let _ = dsh::navigate_main_window(&self.app, &url);
            }
            return;
        }
        let app = self.app.clone();
        let dsh = dsh.clone();
        thread::spawn(move || {
            let Some(bin) = dsh::global_dsh_bin() else {
                log::error!("[upgrade] 返回 dsh:全局 dsh 不可用(可先重试升级),留在升级页");
                return;
            };
            let rx = match dsh::spawn_dsh(&dsh, &bin) {
                Ok(rx) => rx,
                Err(e) => {
                    log::error!("[upgrade] 返回 dsh:启动失败 {e:?},留在升级页");
                    return;
                }
            };
            let (port, rx) = match dsh::wait_ready(&dsh, rx) {
                Ok(p) => p,
                Err(e) => {
                    dsh::kill_child(&dsh);
                    log::error!("[upgrade] 返回 dsh:服务未就绪 {e:?},留在升级页");
                    return;
                }
            };
            let url = format!("http://127.0.0.1:{port}");
            dsh.record_dsh_url(url.clone());
            dsh::spawn_reaper(dsh.clone(), rx);
            log::info!("[upgrade] 返回 dsh:服务已恢复 → {url}");
            let _ = dsh::navigate_main_window(&app, &url);
        });
    }
}

// ── 检测 ───────────────────────────────────────────────────────────

/// 一次检查的核心(启动 / 6h / 手动共用):registry 直查 latest → 与全局已装
/// 版本比较 → 更新托盘徽标/菜单 + 归约状态。任何异常静默按无新版(#2)。
/// 返回结果供手动路径做对话框决策。
async fn run_check(app: &AppHandle) -> CheckResult {
    let installed = dsh::global_dsh_version();
    let Some(latest) = fetch_latest_version().await else {
        // 检查失败:保持托盘/状态现状(徽标不因一次网络抖动消失)
        log::warn!("[upgrade] 检查失败(静默,按无新版)");
        return CheckResult::Failed;
    };
    let Some(up) = app.try_state::<UpgradeManager>() else {
        return CheckResult::Failed;
    };
    match installed.as_deref() {
        Some(cur) if latest_is_newer(&latest, cur) => {
            log::info!("[upgrade] 发现 dsh 新版本 {latest}(当前 {cur})");
            tray::set_dsh_update(app, Some(&latest));
            up.reduce(UpgradeEvent::Found {
                version: latest.clone(),
                current_version: cur.to_string(),
            });
            CheckResult::Found {
                version: latest,
                current_version: cur.to_string(),
            }
        }
        _ => {
            log::info!("[upgrade] dsh 无新版本(当前 {installed:?})");
            tray::set_dsh_update(app, None);
            up.reduce(UpgradeEvent::NoneFound);
            CheckResult::None {
                current_version: installed,
            }
        }
    }
}

pub(crate) enum CheckResult {
    Found {
        version: String,
        current_version: String,
    },
    None {
        current_version: Option<String>,
    },
    Failed,
}

/// registry 直查 latest(abbreviated packument,install-v1)。
/// 3-5s 短超时;超时/非 2xx/解析失败一律 None(静默按无新版)。
async fn fetch_latest_version() -> Option<String> {
    ensure_tls_provider();
    let client = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .ok()?;
    let resp = client
        .get(REGISTRY_URL)
        .header("Accept", "application/vnd.npm.install-v1+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    parse_dist_latest(&body)
}

/// rustls ring provider(与 updater 插件同款):插件在其自身路径懒安装,
/// 本模块的请求可能在它之前发生(启动并发),显式安装保证确定性(幂等)。
fn ensure_tls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// 托盘「检查更新」手动入口的 dsh 层(组合编排见 tray::on_check_update):
/// - boot 未就绪:只亮徽标,不弹对话框(#3 §2:启动未完成不升级,检测照常)
/// - 发现新版 → 徽标 + 对话框 [升级][稍后]
/// - 检查失败 → 「检查更新失败,请稍后重试」对话框
/// - 无新版 → 徽标清除,不弹框(由应用层合并报告「已是最新」)
pub(crate) fn manual_check(app: &AppHandle) -> ManualCheckOutcome {
    let boot_ready = app
        .try_state::<dsh::DshManager>()
        .map(|m| m.inner().phase() == dsh::Phase::Ready)
        .unwrap_or(false);
    let result = tauri::async_runtime::block_on(run_check(app));
    match result {
        CheckResult::Found { version, current_version } if boot_ready => {
            show_found_dialog(app, &version, &current_version);
            ManualCheckOutcome {
                answered: true,
                installed_version: Some(current_version),
            }
        }
        CheckResult::Found { .. } => {
            // boot 进行中:发现新版只亮徽标(不弹对话框,确认也会被守卫拒绝)
            ManualCheckOutcome {
                answered: false,
                installed_version: None,
            }
        }
        CheckResult::Failed => {
            show_check_failed_dialog(app);
            ManualCheckOutcome {
                answered: true,
                installed_version: None,
            }
        }
        CheckResult::None { current_version } => ManualCheckOutcome {
            answered: false,
            installed_version: current_version,
        },
    }
}

pub(crate) struct ManualCheckOutcome {
    /// true = 已用对话框回答(组合入口结束);false = 未弹框(继续应用层检查)
    pub(crate) answered: bool,
    /// 已装 dsh 版本(供合并「已是最新」对话框;未装/未知 = None)
    pub(crate) installed_version: Option<String>,
}

/// 手动检查发现新版:原生对话框 [升级][稍后]。
/// [升级] → 导航升级卡片并自动开始流水线(#3 §1:确认即授权,不二次确认;
/// #3 §4:文案明示中断语义)。
fn show_found_dialog(app: &AppHandle, version: &str, current: &str) {
    let t = locales::shell_texts(locales::detect_lang());
    let app = app.clone();
    app.dialog()
        .message(t.upgrade_found_message(version, current))
        .title("DeepSeek Desktop")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            t.update_now.into(),
            t.update_later.into(),
        ))
        .show_with_result(move |res| {
            if let MessageDialogResult::Custom(s) = res {
                if s == t.update_now {
                    log::info!("[upgrade] 对话框[升级] → 导航升级卡片 + 自动开始流水线");
                    let up = app.state::<UpgradeManager>().inner().clone();
                    let dsh = app.state::<dsh::DshManager>().inner().clone();
                    update::navigate_to_shell(&app);
                    up.confirm_start(&dsh);
                }
            }
        });
}

/// 手动检查失败:原生对话框「检查更新失败,请稍后重试」。
fn show_check_failed_dialog(app: &AppHandle) {
    let t = locales::shell_texts(locales::detect_lang());
    app.dialog()
        .message(t.check_update_failed_message())
        .title("DeepSeek Desktop")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::Ok)
        .show(|_| {});
}

// ── 升级流水线(#3 §2 定稿)──────────────────────────────────────────

/// 独立于 boot 的升级流水线,运行在工作线程;与 boot 的关系是底层工具函数
/// 复用、状态机各自独立(#3 §2:语义/失败语义/阶段都不相同,禁止合并状态机)。
fn upgrade_pipeline(up: &UpgradeManager, dsh: &dsh::DshManager, pin: &str) {
    // 1. killing:杀当前 dsh(UPGRADE_ACTIVE 抑制 reaper 误报,独立于
    //    set_quitting——升级不退出应用,关闭三选对话框保持有效,#3 §2)
    dsh::set_upgrade_active(true);
    up.reduce(UpgradeEvent::PhaseChanged {
        phase: UpgradePhase::Killing,
        progress: None,
        stage: None,
    });
    if !dsh::kill_child_confirm(dsh) {
        log::error!("[upgrade] killing 失败:dsh 未在限定时间内退出");
        dsh::set_upgrade_active(false);
        up.reduce(UpgradeEvent::Failed {
            version: pin.to_string(),
            error: UpgradeError::Kind(UpgradeErrorKind::UpgradeKillFailed {
                detail: format!("{}s 内未退出", dsh::KILL_CONFIRM_TIMEOUT_SECS),
            }),
        });
        return;
    }

    // 2. installing:精确 pin 版本(复用 npm_install_global 与 install_pid
    //    治理,离线缓存同 boot;进度 = 模拟推进,锚点 = 进程退出 + 校准 100%,
    //    逻辑与 boot 共用 install_progress_at / ProgressTicker)
    up.reduce(UpgradeEvent::PhaseChanged {
        phase: UpgradePhase::Installing,
        progress: Some(0),
        stage: Some(dsh::InstallStage::Fetching),
    });
    let ticker = dsh::ProgressTicker::start({
        let up = up.clone();
        move |stage, pct| up.reduce(UpgradeEvent::Progress { progress: pct, stage })
    });
    let result = dsh::npm_install_global(dsh, &format!("@{pin}"));
    ticker.stop_and_join();
    if let Err(e) = result {
        // 安装失败:npm 语义保留旧版(#2 实测);错误以 BootError 形态传播
        // (与 boot 同一 npm 机制同一错误语义,前端统一按 kind 翻译,#3 §3)
        log::error!("[upgrade] 安装失败: {e:?}");
        dsh::set_upgrade_active(false);
        up.reduce(UpgradeEvent::Failed {
            version: pin.to_string(),
            error: UpgradeError::Boot(e),
        });
        return;
    }
    up.reduce(UpgradeEvent::PhaseChanged {
        phase: UpgradePhase::Installing,
        progress: Some(100),
        stage: Some(dsh::InstallStage::Finishing),
    });

    // 3. verify:全局 package.json version == pin 且 bin.js 完整
    up.reduce(UpgradeEvent::PhaseChanged {
        phase: UpgradePhase::Verifying,
        progress: None,
        stage: None,
    });
    let verified = dsh::global_dsh_version().is_some_and(|v| version_eq(&v, pin))
        && dsh::global_dsh_bin().is_some();
    if !verified {
        log::error!("[upgrade] 版本校验失败:目标 {pin}");
        dsh::set_upgrade_active(false);
        up.reduce(UpgradeEvent::Failed {
            version: pin.to_string(),
            error: UpgradeError::Kind(UpgradeErrorKind::UpgradeVerifyFailed),
        });
        return;
    }

    // 4. starting:spawn + wait_ready(新版 bin 路径不变,#2 调研)
    up.reduce(UpgradeEvent::PhaseChanged {
        phase: UpgradePhase::Starting,
        progress: None,
        stage: None,
    });
    let bin = match dsh::global_dsh_bin() {
        Some(b) => b,
        None => {
            // 校验步骤已保证存在,此处仅防御
            dsh::set_upgrade_active(false);
            up.reduce(UpgradeEvent::Failed {
                version: pin.to_string(),
                error: UpgradeError::Kind(UpgradeErrorKind::UpgradeVerifyFailed),
            });
            return;
        }
    };
    let rx = match dsh::spawn_dsh(dsh, &bin) {
        Ok(rx) => rx,
        Err(e) => {
            dsh::set_upgrade_active(false);
            up.reduce(UpgradeEvent::Failed {
                version: pin.to_string(),
                error: UpgradeError::Boot(e),
            });
            return;
        }
    };
    let (port, rx) = match dsh::wait_ready(dsh, rx) {
        Ok(p) => p,
        Err(e) => {
            dsh::kill_child(dsh);
            dsh::set_upgrade_active(false);
            up.reduce(UpgradeEvent::Failed {
                version: pin.to_string(),
                error: UpgradeError::Boot(e),
            });
            return;
        }
    };

    // 5. ready:记录 URL → 清升级抑制标志(旧 dsh 的 reaper 在杀后早已过判定点,
    //    此时清除安全,#3 §2)→ 导航窗口回 dsh 页(新端口 URL)
    let url = format!("http://127.0.0.1:{port}");
    dsh.record_dsh_url(url.clone());
    dsh::spawn_reaper(dsh.clone(), rx);
    dsh::set_upgrade_active(false);
    up.reduce(UpgradeEvent::Succeeded);
    // 升级成功:清托盘徽标/菜单项(已是最新;下一次检查也会确认并清除,但
    // 不等 6h——用户此刻看到的「升级 dsh 到 vX」必须是新状态)
    tray::set_dsh_update(&up.app, None);
    if dsh::navigate_main_window(&up.app, &url) {
        log::info!("[upgrade] 升级完成 → {url}");
        up.reduce(UpgradeEvent::Reset);
    } else {
        // 导航失败:dsh 服务在跑,不杀(返回 dsh 可再导航);错误留卡上可见
        up.reduce(UpgradeEvent::Failed {
            version: pin.to_string(),
            error: UpgradeError::Boot(dsh::BootError::NavigateFailed),
        });
    }
}

// ── Tauri commands(#3 §5:命令面 +3)────────────────────────────────

/// 升级状态快照:前端升级卡片挂载时拉取(先注册监听再 invoke,与 boot-state
/// 同款「后到者覆盖,来自同一状态」竞态语义)。
#[tauri::command]
pub async fn upgrade_state(
    state: tauri::State<'_, UpgradeManager>,
) -> Result<UpgradeStateView, String> {
    Ok(state.inner().snapshot())
}

/// 确认升级(卡片「立即升级/重试」;Rust 侧 boot 阶段 / 状态 / 流水线守卫)。
#[tauri::command]
pub async fn upgrade_confirm(
    state: tauri::State<'_, UpgradeManager>,
    dsh: tauri::State<'_, dsh::DshManager>,
) -> Result<(), String> {
    if state.inner().confirm_start(dsh.inner()) {
        Ok(())
    } else {
        Err("升级被拒:boot 未就绪或升级已在途".into())
    }
}

/// 升级卡片「稍后/返回 dsh」:Rust 侧保证 dsh 在跑(未跑则起当前全局版本),
/// 再导航回 dsh 页(#3 §3,不能「只关卡片」)。
#[tauri::command]
pub async fn upgrade_dismiss(
    state: tauri::State<'_, UpgradeManager>,
    dsh: tauri::State<'_, dsh::DshManager>,
) -> Result<(), String> {
    state.inner().dismiss(dsh.inner());
    Ok(())
}

// ── 测试 ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── registry 响应解析 ─────────────────────────────────────────

    #[test]
    fn parse_dist_latest_extracts_latest_tag() {
        // 生产路径:abbreviated packument(install-v1)的 dist-tags.latest
        assert_eq!(
            parse_dist_latest(r#"{"name":"@deepseek-ai/dsh","dist-tags":{"latest":"0.1.0-rc.6","next":"0.1.0-rc.6"}}"#),
            Some("0.1.0-rc.6".into())
        );
        assert_eq!(
            parse_dist_latest(r#"{"dist-tags":{"latest":"0.2.0"}}"#),
            Some("0.2.0".into())
        );
    }

    #[test]
    fn parse_dist_latest_tolerates_missing_or_garbage() {
        assert_eq!(parse_dist_latest(""), None);
        assert_eq!(parse_dist_latest("not json"), None);
        assert_eq!(parse_dist_latest(r#"{"name":"x"}"#), None); // 无 dist-tags
        assert_eq!(parse_dist_latest(r#"{"dist-tags":{}}"#), None); // 无 latest
        assert_eq!(parse_dist_latest(r#"{"dist-tags":{"latest":123}}"#), None); // 非字符串
    }

    // ── 版本比较 ───────────────────────────────────────────────────

    #[test]
    fn latest_is_newer_orders_numeric_versions() {
        assert!(latest_is_newer("0.2.0", "0.1.0"));
        assert!(latest_is_newer("1.0.0", "0.9.9"));
        assert!(!latest_is_newer("0.1.0", "0.1.0")); // 相等不提示
        assert!(!latest_is_newer("0.1.0", "0.2.0")); // 已装更新不提示(降级场景)
        assert!(latest_is_newer("0.1.10", "0.1.9"));
    }

    #[test]
    fn latest_is_newer_orders_prereleases() {
        // #2 调研:dsh 全为 rc;rc 语义比较必须正确(rc.6 < rc.10)
        assert!(latest_is_newer("0.1.0-rc.6", "0.1.0-rc.5"));
        assert!(latest_is_newer("0.1.0-rc.10", "0.1.0-rc.9"));
        assert!(latest_is_newer("0.1.0-rc.6", "0.0.1-rc.5"));
        assert!(!latest_is_newer("0.1.0-rc.6", "0.1.0-rc.6"));
        // release > prerelease:latest 发 stable 时 rc 用户应被提示(跟随 latest 语义)
        assert!(latest_is_newer("0.1.0", "0.1.0-rc.6"));
        assert!(!latest_is_newer("0.1.0-rc.6", "0.1.0")); // prerelease 不比 release 新
    }

    #[test]
    fn latest_is_newer_normalizes_input() {
        // registry 与 package.json 的版本形态一致(均无 v 前缀),归一化防御
        assert!(latest_is_newer("v0.2.0", "0.1.0"));
        assert!(latest_is_newer("0.2.0", "v0.1.0"));
        assert!(!latest_is_newer(" 0.1.0 ", "0.1.0"));
    }

    #[test]
    fn latest_is_newer_malformed_returns_false() {
        // 解析失败静默按无新版(不为脏数据弹升级提示)
        assert!(!latest_is_newer("not-a-version", "0.1.0"));
        assert!(!latest_is_newer("0.1.0", "not-a-version"));
        assert!(!latest_is_newer("", ""));
        assert!(!latest_is_newer("1.2.3.4", "0.1.0")); // 多余段非法
    }

    #[test]
    fn version_eq_normalizes_before_compare() {
        // 升级后校验:pin 与 package.json version 归一化后逐字比较
        assert!(version_eq("0.1.0-rc.6", "0.1.0-rc.6"));
        assert!(version_eq("v0.1.0-rc.6", "0.1.0-rc.6"));
        assert!(version_eq(" 0.1.0-rc.6 ", "0.1.0-rc.6"));
        assert!(!version_eq("0.1.0-rc.6", "0.1.0-rc.5"));
        assert!(!version_eq("0.1.0-rc.6", "0.1.0"));
    }

    // ── 状态机 ─────────────────────────────────────────────────────

    fn available() -> UpgradeStateView {
        UpgradeStateView::Available {
            version: "0.1.0-rc.6".into(),
            current_version: "0.1.0-rc.3".into(),
        }
    }

    #[test]
    fn check_flow_transitions() {
        // 生产路径:检查 → 发现/无新版 → 状态迁移
        let mut s = UpgradeStateView::Idle;
        s = apply_event(&s, UpgradeEvent::Found {
            version: "0.1.0-rc.6".into(),
            current_version: "0.1.0-rc.3".into(),
        });
        assert_eq!(s, available());
        // 无新版 → 回到 Idle
        s = apply_event(&s, UpgradeEvent::NoneFound);
        assert_eq!(s, UpgradeStateView::Idle);
        // 检查失败不发事件(状态保持;run_check 侧不归约 Failed)
    }

    #[test]
    fn pipeline_flow_transitions() {
        // 生产路径:确认 → 各阶段 → 成功 → 消费完毕
        let mut s = available();
        s = apply_event(&s, UpgradeEvent::Started);
        assert_eq!(
            s,
            UpgradeStateView::Active {
                phase: UpgradePhase::Killing,
                progress: None,
                stage: None
            }
        );
        s = apply_event(&s, UpgradeEvent::PhaseChanged {
            phase: UpgradePhase::Installing,
            progress: Some(0),
            stage: Some(dsh::InstallStage::Fetching),
        });
        s = apply_event(&s, UpgradeEvent::Progress {
            progress: 62,
            stage: dsh::InstallStage::Reifying,
        });
        assert_eq!(
            s,
            UpgradeStateView::Active {
                phase: UpgradePhase::Installing,
                progress: Some(62),
                stage: Some(dsh::InstallStage::Reifying)
            }
        );
        s = apply_event(&s, UpgradeEvent::PhaseChanged {
            phase: UpgradePhase::Verifying,
            progress: None,
            stage: None,
        });
        s = apply_event(&s, UpgradeEvent::PhaseChanged {
            phase: UpgradePhase::Starting,
            progress: None,
            stage: None,
        });
        assert!(matches!(s, UpgradeStateView::Active { phase: UpgradePhase::Starting, .. }));
        s = apply_event(&s, UpgradeEvent::Succeeded);
        assert_eq!(s, UpgradeStateView::Ready);
        // 导航成功后消费完毕
        s = apply_event(&s, UpgradeEvent::Reset);
        assert_eq!(s, UpgradeStateView::Idle);
    }

    #[test]
    fn failure_flow_transitions_and_retry() {
        // 失败 → Failed{version, error};重试 = Started(保留同一 pin)
        let mut s = available();
        s = apply_event(&s, UpgradeEvent::Started);
        s = apply_event(&s, UpgradeEvent::Failed {
            version: "0.1.0-rc.6".into(),
            error: UpgradeError::Kind(UpgradeErrorKind::UpgradeVerifyFailed),
        });
        assert_eq!(
            s,
            UpgradeStateView::Failed {
                version: "0.1.0-rc.6".into(),
                error: UpgradeError::Kind(UpgradeErrorKind::UpgradeVerifyFailed),
            }
        );
        // 重试:Failed → Active(同一 pin,install 幂等且缓存命中快,#3 §3)
        s = apply_event(&s, UpgradeEvent::Started);
        assert!(matches!(s, UpgradeStateView::Active { phase: UpgradePhase::Killing, .. }));
    }

    #[test]
    fn progress_events_ignored_outside_active() {
        // 异常时序不得污染状态(与 boot/update 状态机同款防御)
        let s = available();
        assert_eq!(
            apply_event(&s, UpgradeEvent::Progress {
                progress: 50,
                stage: dsh::InstallStage::Fetching,
            }),
            s
        );
        let s = UpgradeStateView::Idle;
        assert_eq!(
            apply_event(&s, UpgradeEvent::PhaseChanged {
                phase: UpgradePhase::Installing,
                progress: None,
                stage: None,
            }),
            s
        );
    }

    #[test]
    fn check_results_ignored_while_active() {
        // 流水线在途时检查结果不落地(检查在 Active 时本就跳过,此处防御)
        let s = apply_event(&available(), UpgradeEvent::Started);
        assert_eq!(
            apply_event(&s, UpgradeEvent::Found {
                version: "9.9.9".into(),
                current_version: "0.1.0-rc.6".into(),
            }),
            s
        );
        assert_eq!(apply_event(&s, UpgradeEvent::NoneFound), s);
        assert_eq!(apply_event(&s, UpgradeEvent::Reset), s); // 非 Ready 不清
    }

    #[test]
    fn started_rejected_outside_available_or_failed() {
        for s in [
            UpgradeStateView::Idle,
            UpgradeStateView::Ready,
            UpgradeStateView::Active {
                phase: UpgradePhase::Installing,
                progress: None,
                stage: None,
            },
        ] {
            assert_eq!(apply_event(&s, UpgradeEvent::Started), s);
        }
    }

    // ── 序列化契约 ─────────────────────────────────────────────────

    #[test]
    fn upgrade_error_serializes_as_kind_and_data() {
        // 前端 toStructuredError 依赖的线上契约:tag/content 判别式,
        // 字段 camelCase;unit 变体无 data 字段(与 BootError 同形态)
        assert_eq!(
            serde_json::to_value(UpgradeError::Kind(UpgradeErrorKind::UpgradeKillFailed {
                detail: "3s 内未退出".into()
            }))
            .unwrap(),
            serde_json::json!({ "kind": "UpgradeKillFailed", "data": { "detail": "3s 内未退出" } })
        );
        assert_eq!(
            serde_json::to_value(UpgradeError::Kind(UpgradeErrorKind::UpgradeVerifyFailed)).unwrap(),
            serde_json::json!({ "kind": "UpgradeVerifyFailed" })
        );
        // Boot 形态透传:untagged 序列化与 BootError 自身一致,前端零额外机制
        assert_eq!(
            serde_json::to_value(UpgradeError::Boot(dsh::BootError::DshStartTimeout { seconds: 180 }))
                .unwrap(),
            serde_json::json!({ "kind": "DshStartTimeout", "data": { "seconds": 180 } })
        );
    }

    #[test]
    fn upgrade_state_view_serializes_with_status_tag() {
        // upgrade-state 事件 / upgrade_state 命令的线上契约:内部 tag status,
        // 字段 camelCase;Active 带 phase,progress/stage 仅 installing 携带
        assert_eq!(
            serde_json::to_value(UpgradeStateView::Idle).unwrap(),
            serde_json::json!({ "status": "idle" })
        );
        assert_eq!(
            serde_json::to_value(available()).unwrap(),
            serde_json::json!({
                "status": "available",
                "version": "0.1.0-rc.6",
                "currentVersion": "0.1.0-rc.3"
            })
        );
        assert_eq!(
            serde_json::to_value(UpgradeStateView::Active {
                phase: UpgradePhase::Installing,
                progress: Some(62),
                stage: Some(dsh::InstallStage::Reifying),
            })
            .unwrap(),
            serde_json::json!({
                "status": "active",
                "phase": "installing",
                "progress": 62,
                "stage": "reifying"
            })
        );
        // 非 installing 阶段不带 progress/stage 字段
        assert_eq!(
            serde_json::to_value(UpgradeStateView::Active {
                phase: UpgradePhase::Starting,
                progress: None,
                stage: None,
            })
            .unwrap(),
            serde_json::json!({ "status": "active", "phase": "starting" })
        );
        assert_eq!(
            serde_json::to_value(UpgradeStateView::Ready).unwrap(),
            serde_json::json!({ "status": "ready" })
        );
        assert_eq!(
            serde_json::to_value(UpgradeStateView::Failed {
                version: "0.1.0-rc.6".into(),
                error: UpgradeError::Boot(dsh::BootError::DshExitedEarly { exit_code: 1 }),
            })
            .unwrap(),
            serde_json::json!({
                "status": "failed",
                "version": "0.1.0-rc.6",
                "error": { "kind": "DshExitedEarly", "data": { "exitCode": 1 } }
            })
        );
    }
}
