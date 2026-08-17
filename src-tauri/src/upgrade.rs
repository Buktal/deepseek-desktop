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
//! (tooltip 同步),不弹窗打断;手动检查 → 原生对话框直接回答([升级] → 导航
//! 升级卡片并自动开始流水线,#3:确认即授权,不二次确认)。
//!
//! 流水线(#3 §2 定稿,独立于 boot 状态机;底层工具全部复用 dsh.rs,
//! 禁止复制实现):
//!   confirm → killing(杀当前 dsh,UPGRADE_ACTIVE 抑制 reaper 误报)
//!          → installing(`npm install -g @deepseek-ai/dsh@<pin>`,复用
//!            npm_install_global 与 install_pid 治理;进度模拟复用
//!            install_progress_at / ProgressTicker,锚点 = 进程退出 + 校准 100%)
//!          → verify(读全局 package.json version == pin 且 bin.js 完整)
//!          → starting(spawn_dsh + wait_ready,新版 bin 路径不变)
//!          → ready(record_dsh_url 推新端口 URL 给壳页,iframe 自动切换,#36)
//!
//! 失败处理(#3 §3):失败保留旧版(npm 语义,#2 实测)+ 恢复服务——
//! [返回 dsh] 经 upgrade_dismiss:Rust 侧检查 dsh 是否在运行,未运行则起当前
//! 全局安装(失败时旧版保留/新版已装好,都是「当前全局版本」)→ 就绪 →
//! record_dsh_url 推 URL → 卡片关闭(Dismissed → Idle),不能「只关卡片」。
//! 错误结构化:升级特有 kind(UpgradeKillFailed / UpgradeVerifyFailed)独立枚举,
//! 安装/启动类直接以 DshError 形态传播,前端统一按 kind 翻译
//! (errors.<kind> 键,零额外机制)。
//!
//! 壳页常驻(ADR 0001 / #36)后卡片是浮层:可见性由状态驱动(active/ready/
//! failed 必显,available 需托盘显式请求,见前端 isUpgradeCardVisible);
//! 托盘「升级 dsh 到 vX」菜单点击不再导航本地页,改为显示窗口 + 推
//! `upgrade-card-request` 事件(tray.rs)。

use std::cmp::Ordering;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::error::{DshError, UpgradeError, UpgradeErrorKind};
use crate::{dsh, npm, tray, update};

/// registry abbreviated packument(install-v1 Accept 头,23KB,#2 调研实测)。
const REGISTRY_URL: &str = "https://registry.npmjs.org/@deepseek-ai/dsh";
/// 直查超时(#2 定稿 3-5s):超时/任何异常静默按无新版,不影响启动。
const CHECK_TIMEOUT: Duration = Duration::from_secs(4);
/// 帧嵌入回归检查超时(本地 127.0.0.1 服务,3s 足够)。
const FRAME_CHECK_TIMEOUT: Duration = Duration::from_secs(3);

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
        stage: Option<npm::InstallStage>,
    },
    /// 升级成功(瞬态:Rust 随即导航回 dsh 页)
    Ready,
    /// 升级失败(旧版保留 + 恢复服务;#3 §3)。
    /// version = 目标 pin(重试继续用同一版本,install 幂等且缓存命中快)
    Failed { version: String, error: UpgradeError },
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
        stage: Option<npm::InstallStage>,
    },
    /// 安装进度(模拟推进;100 只能由 npm 进程退出校准)
    Progress { progress: u8, stage: npm::InstallStage },
    /// 流水线成功(→ Ready)
    Succeeded,
    /// 流水线失败(→ Failed{version, error})
    Failed { version: String, error: UpgradeError },
    /// 升级链消费完毕(Ready → Idle)
    Reset,
    /// 卡片「稍后/返回 dsh」关闭卡片(Available/Failed → Idle;
    /// 壳页常驻后卡片是浮层,关闭 = 状态归位,#36)
    Dismissed,
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
        UpgradeEvent::Dismissed => match state {
            UpgradeStateView::Available { .. } | UpgradeStateView::Failed { .. } => {
                UpgradeStateView::Idle
            }
            _ => state.clone(),
        },
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
        let was_running = self.is_pipeline_running();
        let next = {
            let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let next = apply_event(&s, event);
            *s = next.clone();
            next
        };
        log::info!("[upgrade] state → {next:?}");
        let _ = self.app.emit_to("main", "upgrade-state", next);
        // 流水线启停跨界时刷新菜单:「检查更新」disabled 随快照同步
        // (Started 进入 Active 置灰,成功/失败离开 Active 恢复;#38;
        // 成功路径的 set_dsh_update(None) 也会刷新一次,幂等无害)
        if was_running != self.is_pipeline_running() {
            tray::refresh_menu(&self.app);
        }
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
                    error: UpgradeError::Dsh(DshError::Internal { message: msg }),
                });
            }
        });
        true
    }

    /// 升级卡片「稍后/返回 dsh」(#3 §3):Rust 侧保证 dsh 在跑——未运行则起
    /// 当前全局安装(失败保留旧版 / 新版已装好,都是「当前全局版本」)→ 就绪 →
    /// record_dsh_url 推 URL 给壳页(iframe 自动切换)→ 卡片关闭(Dismissed)。
    /// 不能「只关卡片」,否则用户回到打不开的页面。
    /// 壳页常驻(ADR 0001 / #36):dsh 在跑时 iframe 仍指向它,无需任何动作,
    /// 只关卡片。
    pub fn dismiss(&self, dsh: &dsh::DshManager) {
        if dsh::dsh_is_running(dsh) {
            log::info!("[upgrade] 返回 dsh(dsh 仍在运行),关闭升级卡片");
            self.reduce(UpgradeEvent::Dismissed);
            return;
        }
        let up = self.clone();
        let dsh = dsh.clone();
        thread::spawn(move || {
            let Some(bin) = npm::global_dsh_bin() else {
                log::error!("[upgrade] 返回 dsh:全局 dsh 不可用(可先重试升级),留在升级卡片");
                return;
            };
            let rx = match dsh::spawn_dsh(&dsh, &bin) {
                Ok(rx) => rx,
                Err(e) => {
                    log::error!("[upgrade] 返回 dsh:启动失败 {e:?},留在升级卡片");
                    return;
                }
            };
            let (port, rx) = match dsh::wait_ready(&dsh, rx) {
                Ok(p) => p,
                Err(e) => {
                    dsh::kill_child(&dsh);
                    log::error!("[upgrade] 返回 dsh:服务未就绪 {e:?},留在升级卡片");
                    return;
                }
            };
            let url = dsh::dsh_url_for_port(port);
            dsh.record_dsh_url(url.clone());
            dsh::spawn_reaper(dsh.clone(), rx);
            log::info!("[upgrade] 返回 dsh:服务已恢复 → {url}");
            up.reduce(UpgradeEvent::Dismissed);
        });
    }
}

// ── 检测 ───────────────────────────────────────────────────────────

/// 一次检查的核心(启动 / 6h / 手动共用):registry 直查 latest → 与全局已装
/// 版本比较 → 更新托盘徽标/菜单 + 归约状态。任何异常静默按无新版(#2)。
/// 返回结果供手动路径做对话框决策。
async fn run_check(app: &AppHandle) -> CheckResult {
    let installed = npm::global_dsh_version();
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

// ── 上游耦合防线:帧嵌入回归检查(ADR 0001 / #29,#41)────────────────

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
/// 命中 XFO / frame-ancestors → Err(UpgradeFrameBlocked,升级流水线失败,
/// 前端按 errors.UpgradeFrameBlocked 翻译并指引回退预案)。
/// 请求失败/超时/客户端构造失败 → 记日志放行:探测不确定不等于「被禁止」,
/// 不为不确定的探测拦掉升级(防御检查是找「已确认的上游耦合」,不是网络
/// 可用性检查;wait_ready 已确认服务在监听)。
async fn check_frame_blocking(url: &str) -> Result<(), UpgradeError> {
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
            log::warn!("[upgrade] 帧嵌入回归检查:请求失败(按未命中放行) {url}: {e}");
            return Ok(());
        }
    };
    if let Some(header) = frame_blocking_header(resp.headers()) {
        log::error!("[upgrade] 帧嵌入回归检查:命中 {header} → 升级失败");
        return Err(UpgradeError::Kind(UpgradeErrorKind::UpgradeFrameBlocked {
            header,
        }));
    }
    log::info!("[upgrade] 帧嵌入回归检查通过(无 XFO / frame-ancestors) {url}");
    Ok(())
}

/// 托盘「检查更新」手动入口的 dsh 层(组合编排与弹窗/toast 呈现全在
/// tray::on_check_update,#39):检查 → 结果。boot 未就绪的判定由编排方做
/// (tray 读 DshManager phase)——本模块只负责「检查」职责,UI 决策不内嵌。
pub(crate) fn manual_check(app: &AppHandle) -> CheckResult {
    tauri::async_runtime::block_on(run_check(app))
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
        stage: Some(npm::InstallStage::Fetching),
    });
    let ticker = npm::ProgressTicker::start({
        let up = up.clone();
        move |stage, pct| up.reduce(UpgradeEvent::Progress { progress: pct, stage })
    });
    let result = dsh::npm_install_global(dsh, &format!("@{pin}"));
    ticker.stop_and_join();
    if let Err(e) = result {
        // 安装失败:npm 语义保留旧版(#2 实测);错误以 DshError 形态传播
        // (与 boot 同一 npm 机制同一错误语义,前端统一按 kind 翻译,#3 §3)
        log::error!("[upgrade] 安装失败: {e:?}");
        dsh::set_upgrade_active(false);
        up.reduce(UpgradeEvent::Failed {
            version: pin.to_string(),
            error: UpgradeError::Dsh(e),
        });
        return;
    }
    up.reduce(UpgradeEvent::PhaseChanged {
        phase: UpgradePhase::Installing,
        progress: Some(100),
        stage: Some(npm::InstallStage::Finishing),
    });

    // 3. verify:全局 package.json version == pin 且 bin.js 完整
    up.reduce(UpgradeEvent::PhaseChanged {
        phase: UpgradePhase::Verifying,
        progress: None,
        stage: None,
    });
    let verified = npm::global_dsh_version().is_some_and(|v| version_eq(&v, pin))
        && npm::global_dsh_bin().is_some();
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
    let bin = match npm::global_dsh_bin() {
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
                error: UpgradeError::Dsh(e),
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
                error: UpgradeError::Dsh(e),
            });
            return;
        }
    };

    // 4.5 上游耦合防线(ADR 0001 / #29,#41):回归检查新版 dsh 的响应头——
    // XFO / CSP frame-ancestors 命中 = iframe 架构无法呈现它,升级报明确错误
    // (指引回退预案 = 恢复整窗互斥导航,git 历史可回)。注意此失败模式
    // npm install 已成功、旧版已被替换(与「保留旧版」的安装失败语义不同),
    // 错误文案由前端按 kind 翻译,不在此拼装。
    if let Err(e) = tauri::async_runtime::block_on(check_frame_blocking(
        &dsh::dsh_url_for_port(port),
    )) {
        dsh::kill_child(dsh);
        dsh::set_upgrade_active(false);
        up.reduce(UpgradeEvent::Failed {
            version: pin.to_string(),
            error: e,
        });
        return;
    }

    // 5. ready:record_dsh_url 推新端口 URL 给壳页(iframe 自动切换,ADR 0001)
    //    → 清升级抑制标志(旧 dsh 的 reaper 在杀后早已过判定点,此时清除安全,
    //    #3 §2)→ Ready 瞬态立即消费(壳页常驻后无窗口导航,卡片随 Idle 关闭)
    let url = dsh::dsh_url_for_port(port);
    dsh.record_dsh_url(url.clone());
    dsh::spawn_reaper(dsh.clone(), rx);
    dsh::set_upgrade_active(false);
    up.reduce(UpgradeEvent::Succeeded);
    // 升级成功:清托盘徽标/菜单项(已是最新;下一次检查也会确认并清除,但
    // 不等 6h——用户此刻看到的「升级 dsh 到 vX」必须是新状态)
    tray::set_dsh_update(&up.app, None);
    up.reduce(UpgradeEvent::Reset);
    log::info!("[upgrade] 升级完成,已推 URL 给壳页 → {url}");
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

    // ── 帧嵌入回归检查(上游耦合防线,ADR 0001 / #41)──────────────

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
        // 验收(issue #41):本地起一个带 XFO 头的假 dsh 服务,升级检查能报错。
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

        // 带 X-Frame-Options: DENY 的假服务 → 检查报 UpgradeFrameBlocked
        let (url, h) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nX-Frame-Options: DENY\r\nContent-Length: 4\r\n\r\n<h1>x</h1>"
                .to_string(),
        );
        let err = tauri::async_runtime::block_on(check_frame_blocking(&url)).unwrap_err();
        assert_eq!(
            err,
            UpgradeError::Kind(UpgradeErrorKind::UpgradeFrameBlocked {
                header: "X-Frame-Options: DENY".into()
            })
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
            stage: Some(npm::InstallStage::Fetching),
        });
        s = apply_event(&s, UpgradeEvent::Progress {
            progress: 62,
            stage: npm::InstallStage::Reifying,
        });
        assert_eq!(
            s,
            UpgradeStateView::Active {
                phase: UpgradePhase::Installing,
                progress: Some(62),
                stage: Some(npm::InstallStage::Reifying)
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
                stage: npm::InstallStage::Fetching,
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
    fn dismissed_closes_card_from_available_and_failed() {
        // 卡片「稍后/返回 dsh」(壳页常驻后关闭卡片 = 状态归位,#36)
        let s = apply_event(&available(), UpgradeEvent::Dismissed);
        assert_eq!(s, UpgradeStateView::Idle);
        let mut s = available();
        s = apply_event(&s, UpgradeEvent::Started);
        s = apply_event(&s, UpgradeEvent::Failed {
            version: "0.1.0-rc.6".into(),
            error: UpgradeError::Kind(UpgradeErrorKind::UpgradeVerifyFailed),
        });
        let s = apply_event(&s, UpgradeEvent::Dismissed);
        assert_eq!(s, UpgradeStateView::Idle);
        // 流水线在途 / Ready 不因 Dismissed 归位(Reset 负责 Ready 的消费)
        let s = apply_event(&available(), UpgradeEvent::Started);
        assert_eq!(apply_event(&s, UpgradeEvent::Dismissed), s);
        let s = apply_event(&available(), UpgradeEvent::Started);
        let s = apply_event(&s, UpgradeEvent::Succeeded);
        assert_eq!(apply_event(&s, UpgradeEvent::Dismissed), s);
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
                stage: Some(npm::InstallStage::Reifying),
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
                error: UpgradeError::Dsh(DshError::DshExitedEarly { exit_code: 1 }),
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
