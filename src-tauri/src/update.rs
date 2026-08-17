//! 应用自身升级(tauri-plugin-updater):常驻检查 + 状态机 + 下载安装 + relaunch。
//!
//! 架构(#9 重审定稿):本外壳的 dsh 页是 remote origin,收不到任何 Tauri 事件/命令,
//! O_CC_One 的前端常驻 update-check hook(启动探测 + 6h 轮询)在本外壳不适用——
//! 检查/下载/安装全部在 Rust 侧,前端只是本地页上的镜像视图(挂载时拉 `update_state`
//! 快照 + 监听 `update-state` 事件,与 boot-state 同款竞态语义)。
//!
//! 检测时机(#1 定稿,两层升级共用):启动探测 + 6h 轮询 + 托盘手动入口(tray.rs)。
//! 检测失败静默:按无新版处理,不进错误态、不弹窗。
//!
//! 通知形态(#3 定稿,与 dsh 升级共用同一 Rust 侧机制):自动检测(启动/6h)发现新版
//! → 托盘徽标图标变体 + 动态菜单项「升级到 vX」+ tooltip,不弹窗打断;托盘手动检查
//! → 原生对话框直接回答(已是最新 / 发现新版本 [升级][稍后])。点击动态菜单项 →
//! 显示窗口 + 推 `update-card-request` 事件,前端按状态渲染升级卡片(壳页常驻,
//! 卡片是浮层,#36;可见性规则见前端 isUpdateCardVisible)。
//!
//! 升级时序(#9 定稿):检查/下载期间 dsh 照常运行;下载完成且签名校验通过后、
//! 安装器启动之前,必须先 `set_quitting()` + `kill_child()`——Windows NSIS
//! passive 模式(/P /R /UPDATE)的安装器会在安装完成后自动重启应用,若不在安装前
//! 杀 dsh,旧 dsh 会残留成孤儿,且 reaper 会把强制杀掉的 dsh 判为「意外退出」弹窗。
//! 注意:#3 的 UPGRADE_ACTIVE 独立抑制标志是 dsh 升级(杀旧 dsh 但保留关闭对话框)
//! 的设计,应用自身升级按 #9 执行(set_quitting 放行 CloseRequested 是退出路径
//! 语义,进程随即重启,无副作用),两者不冲突。
//!
//! 状态机在 Rust(单一事实源),`update-state` 事件推给 main 窗口(仅本地页能收)。
//! 归约是纯函数 `apply_event`,生产路径唯一入口是 `UpdateManager::reduce`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::Emitter;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::error::UpdateError;
use crate::{dsh, tray};

/// 6h 轮询间隔(#1 定稿:启动 + 定时 6h + 托盘手动,两层升级共用)。
/// upgrade.rs(dsh 升级链)复用同一常量,保证两层触发时机一致(单一事实来源)。
pub(crate) const POLL_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

// ── 全局守卫(跨线程)────────────────────────────────────────────────

/// 检查在途标志:启动探测与手动点击撞车时后者 no-op(并发保护)。
static CHECKING: AtomicBool = AtomicBool::new(false);
/// 下载/安装在途标志:卡片按钮双击等防重入。
static APPLYING: AtomicBool = AtomicBool::new(false);

// ── 状态与视图 ─────────────────────────────────────────────────────

/// 升级状态快照(`update-state` 事件与 `update_state` 命令共用)。
/// serde 序列化为 `{"status":"available","version":...,"currentVersion":...,"notes":...}`
/// (内部 tag),unit 变体无多余字段;前端按 status 分发渲染。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status", rename_all_fields = "camelCase")]
pub enum UpdateStateView {
    /// 无更新 / 检测失败(静默按无新版)
    Idle,
    /// 检查在途(前端不渲染此态,仅防手动入口重入)
    Checking,
    /// 发现新版,等待用户操作
    Available {
        version: String,
        current_version: String,
        /// 最新版 release notes(latest.json `notes`,可为空)
        notes: Option<String>,
    },
    /// 下载/安装中
    Downloading {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    /// 已安装,等待重启
    Ready,
    /// 下载/安装失败 → 降级 GitHub 手动下载
    Failed { error: UpdateError },
}

/// 状态机事件。由 update.rs 各动作(检查/下载/安装/卡片关闭)产生,经
/// `apply_event` 归约。
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateEvent {
    CheckStarted,
    /// 检查发现新版(version/currentVersion/notes 来自 latest.json)
    CheckFound {
        version: String,
        current_version: String,
        notes: Option<String>,
    },
    /// 检查无新版,或检查失败(静默)
    CheckNone,
    DownloadStarted { total_bytes: u64 },
    DownloadProgress {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    DownloadFinished,
    DownloadFailed { detail: String },
    /// 卡片「稍后/关闭」(Available/Ready/Failed → Idle)
    Dismissed,
}

/// 状态机归约:事件 → 新状态。纯函数,可测;
/// 生产路径唯一调用方是 `UpdateManager::reduce`(先归约后下发,不跑分叉逻辑)。
pub fn apply_event(state: &UpdateStateView, event: UpdateEvent) -> UpdateStateView {
    match event {
        UpdateEvent::CheckStarted => UpdateStateView::Checking,
        UpdateEvent::CheckFound {
            version,
            current_version,
            notes,
        } => UpdateStateView::Available {
            version,
            current_version,
            notes,
        },
        UpdateEvent::CheckNone => UpdateStateView::Idle,
        UpdateEvent::DownloadStarted { total_bytes } => UpdateStateView::Downloading {
            downloaded_bytes: 0,
            total_bytes,
        },
        UpdateEvent::DownloadProgress {
            downloaded_bytes,
            total_bytes,
        } => {
            // 进度事件仅对下载态生效(其余态忽略,防异常时序污染状态)
            if matches!(state, UpdateStateView::Downloading { .. }) {
                UpdateStateView::Downloading {
                    downloaded_bytes,
                    total_bytes,
                }
            } else {
                state.clone()
            }
        }
        UpdateEvent::DownloadFinished => UpdateStateView::Ready,
        UpdateEvent::DownloadFailed { detail } => UpdateStateView::Failed {
            error: UpdateError::DownloadFailed { detail },
        },
        UpdateEvent::Dismissed => match state {
            // 卡片可关闭态(有「稍后/关闭」按钮):Available / Ready / Failed;
            // 下载中不可关闭(卡片无此按钮),异常时序不得污染状态
            UpdateStateView::Available { .. }
            | UpdateStateView::Ready
            | UpdateStateView::Failed { .. } => UpdateStateView::Idle,
            _ => state.clone(),
        },
    }
}

/// 升级管理器。Clone 共享内部状态(检查线程/下载线程各持一份)。
#[derive(Clone)]
pub struct UpdateManager {
    app: AppHandle,
    state: Arc<Mutex<UpdateStateView>>,
    /// 最近一次 check 找到的 Update(`download_and_install` 的载体,非序列化,
    /// 跨线程共享;下载/安装必须用「同一次 check 的结果」,不能重查)
    pending: Arc<Mutex<Option<Update>>>,
}

impl UpdateManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            state: Arc::new(Mutex::new(UpdateStateView::Idle)),
            pending: Arc::new(Mutex::new(None)),
        }
    }

    pub fn snapshot(&self) -> UpdateStateView {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 状态归约 + 下发(生产路径唯一入口,测试的 apply_event 即此处的归约)。
    fn reduce(&self, event: UpdateEvent) {
        let next = {
            let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let next = apply_event(&s, event);
            *s = next.clone();
            next
        };
        log::info!("[update] state → {next:?}");
        let _ = self.app.emit_to("main", "update-state", next);
    }

    /// 流水线在途(下载/已就绪)时拒绝新检查(#3:流水线运行中手动入口 no-op;
    /// tray.rs 菜单 disabled 与 toast 守卫同源读取,#39)。
    pub(crate) fn is_active(&self) -> bool {
        matches!(
            self.snapshot(),
            UpdateStateView::Downloading { .. } | UpdateStateView::Ready
        )
    }

    /// 常驻检查(setup 调用一次):启动探测 + 6h 轮询。
    /// 探测失败静默(按无新版),不影响启动。
    pub fn start_resident_checks(&self) {
        self.check_now(false, None);
        let m = self.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(POLL_INTERVAL);
            m.check_now(false, None);
        });
    }

    /// 检查更新。manual=true(托盘「检查更新」组合入口,#17)时结果经 on_done
    /// 回调送出(对话框决策在编排方 tray::on_check_update 统一进行,避免两层
    /// 升级各自弹框叠加);自动检测静默。并发保护:CHECKING 防重入;流水线在途
    /// 时 no-op(注意:CHECKING 在途时提前返回,on_done 不会被调用)。
    pub fn check_now(&self, manual: bool, on_done: Option<Box<dyn FnOnce(ManualCheckResult) + Send>>) {
        if self.is_active() {
            log::info!("[update] 升级流水线在途,跳过检查");
            return;
        }
        if CHECKING.swap(true, Ordering::SeqCst) {
            log::info!("[update] 检查已在进行,跳过");
            return;
        }
        let m = self.clone();
        tauri::async_runtime::spawn(async move {
            m.reduce(UpdateEvent::CheckStarted);
            let updater = match m.app.updater() {
                Ok(u) => u,
                Err(e) => {
                    log::warn!("[update] updater 不可用(静默): {e}");
                    CHECKING.store(false, Ordering::SeqCst);
                    m.reduce(UpdateEvent::CheckNone);
                    if manual {
                        if let Some(f) = on_done {
                            f(ManualCheckResult::Failed);
                        }
                    }
                    return;
                }
            };
            let result = updater.check().await;
            CHECKING.store(false, Ordering::SeqCst);
            match result {
                Ok(Some(update)) => {
                    log::info!(
                        "[update] 发现新版本 {} (当前 {})",
                        update.version,
                        update.current_version
                    );
                    m.pending.lock().unwrap_or_else(|p| p.into_inner()).replace(update.clone());
                    m.reduce(UpdateEvent::CheckFound {
                        version: update.version.clone(),
                        current_version: update.current_version.clone(),
                        notes: update.body.clone(),
                    });
                    tray::set_app_update(&m.app, Some(&update.version));
                    if manual {
                        if let Some(f) = on_done {
                            f(ManualCheckResult::Found {
                                version: update.version,
                                current_version: update.current_version,
                                // notes 随结果携带(tray.rs 编排弹窗时下发原文,
                                // 前端 summarizeReleaseNotes 复用,#39)
                                notes: update.body.clone(),
                            });
                        }
                    }
                }
                Ok(None) => {
                    log::info!("[update] 已是最新版本");
                    m.reduce(UpdateEvent::CheckNone);
                    tray::set_app_update(&m.app, None);
                    if manual {
                        if let Some(f) = on_done {
                            f(ManualCheckResult::None);
                        }
                    }
                }
                Err(e) => {
                    // 检测失败静默(#1):不进错误态、不弹窗,等下一次触发
                    log::warn!("[update] 检查失败(静默): {e}");
                    m.reduce(UpdateEvent::CheckNone);
                    tray::set_app_update(&m.app, None);
                    if manual {
                        if let Some(f) = on_done {
                            f(ManualCheckResult::Failed);
                        }
                    }
                }
            }
        });
    }

    /// 下载并安装(卡片「立即更新」/ 手动检查对话框「升级」)。
    /// 并发保护:APPLYING 防重入。进度经 `update-state` 事件推送。
    ///
    /// 时序(#9 定稿的落点):下载期间 dsh 照常运行;下载完成且**签名校验通过**
    /// 后、安装器启动之前,先 set_quitting + kill_child——Windows NSIS passive
    /// 模式(/P /R /UPDATE)的安装器会在安装完成后自动重启应用,若不在安装前杀掉
    /// dsh,旧 dsh 会残留成孤儿,且 reaper 会把强制杀掉的 dsh 判为「意外退出」弹窗。
    /// 分两步走(download 与 install 拆开):`download()` 内部完成签名校验,
    /// 校验失败返回 Err,此时 dsh 必须仍在运行(用户会话不得无谓中断);
    /// 只有校验通过才进入杀进程 → 安装。
    pub fn apply_now(&self, dsh: &dsh::DshManager) {
        if APPLYING.swap(true, Ordering::SeqCst) {
            log::info!("[update] 下载/安装已在进行,跳过");
            return;
        }
        let Some(update) = self.pending.lock().unwrap_or_else(|p| p.into_inner()).clone()
        else {
            APPLYING.store(false, Ordering::SeqCst);
            log::warn!("[update] 无待安装的更新(apply 被拒)");
            return;
        };
        let m = self.clone();
        let dsh = dsh.clone();
        tauri::async_runtime::spawn(async move {
            let mut downloaded = 0u64;
            let mut total = 0u64;
            m.reduce(UpdateEvent::DownloadStarted { total_bytes: 0 });
            let result = update
                .download(
                    |len, content_length| {
                        total = content_length.unwrap_or(0);
                        downloaded += len as u64;
                        m.reduce(UpdateEvent::DownloadProgress {
                            downloaded_bytes: downloaded,
                            total_bytes: total,
                        });
                    },
                    || {},
                )
                .await
                .and_then(|bytes| {
                    // 下载完成且签名校验通过:安装器即将启动(NSIS /R 自动重启应用),
                    // relaunch 前必须先杀 dsh(#9 时序)
                    log::info!("[update] 安装前杀 dsh 子进程(NSIS /R 将自动重启应用)…");
                    dsh::set_quitting();
                    dsh::kill_child(&dsh);
                    update.install(bytes)
                });
            APPLYING.store(false, Ordering::SeqCst);
            match result {
                Ok(()) => {
                    log::info!("[update] 安装完成,等待重启");
                    m.reduce(UpdateEvent::DownloadFinished);
                }
                Err(e) => {
                    // 失败降级:卡片展示结构化错误 + GitHub 手动下载入口
                    log::error!("[update] 下载/安装失败: {e}");
                    m.reduce(UpdateEvent::DownloadFailed { detail: e.to_string() });
                }
            }
        });
    }

    /// 升级就绪后重启应用(#9 时序):relaunch 前必须先 set_quitting + kill_child
    /// ——否则旧 dsh 残留成孤儿,且 reaper 会把强制杀掉的 dsh 判为「意外退出」
    /// 弹窗挡住 relaunch。set_quitting 放行 CloseRequested 是退出路径语义,
    /// 进程随即重启,无副作用。app.restart() 返回 `!`,本函数实际不返回。
    pub fn restart_now(&self, dsh: &dsh::DshManager) {
        log::info!("[update] relaunch 前杀 dsh 子进程…");
        dsh::set_quitting();
        dsh::kill_child(dsh);
        self.app.restart();
    }

    /// 升级卡片「稍后/关闭」:关闭卡片(状态归 Idle)。
    /// 壳页常驻(ADR 0001 / #36)后无窗口导航——dsh 由 iframe 呈现,不受应用
    /// 升级影响,「关闭」只是收起浮层。
    pub fn dismiss(&self) {
        self.reduce(UpdateEvent::Dismissed);
    }
}

// ── 手动检查结果(供组合入口使用:tray::on_check_update 编排两层回答,
// 弹窗/toast 由 dialog.rs 呈现,#39)──────────────────────────────────

/// 手动检查的结果。Found 携带 notes(弹窗正文摘要的原文来源)。
pub enum ManualCheckResult {
    Found {
        version: String,
        current_version: String,
        notes: Option<String>,
    },
    None,
    Failed,
}

// ── Tauri commands ─────────────────────────────────────────────────

/// 升级状态快照:前端升级卡片挂载时拉取(先注册监听再 invoke,与 boot-state
/// 同款「后到者覆盖,来自同一状态」竞态语义)。
#[tauri::command]
pub async fn update_state(state: tauri::State<'_, UpdateManager>) -> Result<UpdateStateView, String> {
    Ok(state.inner().snapshot())
}

/// 开始下载并安装(卡片「立即更新」;Rust 侧 APPLYING 守卫防重入)。
#[tauri::command]
pub async fn update_apply(
    state: tauri::State<'_, UpdateManager>,
    dsh: tauri::State<'_, dsh::DshManager>,
) -> Result<(), String> {
    state.inner().apply_now(dsh.inner());
    Ok(())
}

/// 升级就绪后重启应用:先杀 dsh 子进程再 relaunch(#9 时序)。
/// app.restart() 触发进程重启,本命令实际不返回(调用方无需等待响应)。
#[tauri::command]
pub async fn update_restart(
    state: tauri::State<'_, UpdateManager>,
    dsh: tauri::State<'_, dsh::DshManager>,
) -> Result<(), String> {
    state.inner().restart_now(dsh.inner());
    // 不可达:restart_now 内 app.restart() 触发进程重启,本命令不返回
    Ok(())
}

/// 升级卡片「稍后/关闭」:关闭卡片(状态归 Idle,壳页常驻后无导航,#36)。
#[tauri::command]
pub async fn update_dismiss(state: tauri::State<'_, UpdateManager>) -> Result<(), String> {
    state.inner().dismiss();
    Ok(())
}

// ── 测试 ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 Available 状态的便捷函数(测试数据)。
    fn available() -> UpdateStateView {
        UpdateStateView::Available {
            version: "0.2.0".into(),
            current_version: "0.1.0".into(),
            notes: None,
        }
    }

    #[test]
    fn check_flow_transitions() {
        // 生产路径:检查 → 发现/无新版 → 状态迁移
        let mut s = UpdateStateView::Idle;
        s = apply_event(&s, UpdateEvent::CheckStarted);
        assert_eq!(s, UpdateStateView::Checking);
        s = apply_event(&s, UpdateEvent::CheckFound {
            version: "0.2.0".into(),
            current_version: "0.1.0".into(),
            notes: None,
        });
        assert_eq!(s, available());
        // 无新版 → 回到 Idle(检测失败同此,静默)
        let mut s2 = apply_event(&s, UpdateEvent::CheckStarted);
        s2 = apply_event(&s2, UpdateEvent::CheckNone);
        assert_eq!(s2, UpdateStateView::Idle);
    }

    #[test]
    fn download_flow_transitions() {
        // 生产路径:下载 → 进度 → 完成/失败
        let mut s = available();
        s = apply_event(&s, UpdateEvent::DownloadStarted { total_bytes: 100 });
        assert_eq!(
            s,
            UpdateStateView::Downloading {
                downloaded_bytes: 0,
                total_bytes: 100
            }
        );
        s = apply_event(
            &s,
            UpdateEvent::DownloadProgress {
                downloaded_bytes: 40,
                total_bytes: 100,
            },
        );
        assert_eq!(
            s,
            UpdateStateView::Downloading {
                downloaded_bytes: 40,
                total_bytes: 100
            }
        );
        s = apply_event(&s, UpdateEvent::DownloadFinished);
        assert_eq!(s, UpdateStateView::Ready);

        // 失败路径(与完成路径互斥)
        let mut f = apply_event(&available(), UpdateEvent::DownloadStarted { total_bytes: 0 });
        f = apply_event(
            &f,
            UpdateEvent::DownloadFailed { detail: "network error".into() },
        );
        assert_eq!(
            f,
            UpdateStateView::Failed {
                error: UpdateError::DownloadFailed {
                    detail: "network error".into()
                }
            }
        );
    }

    #[test]
    fn progress_outside_downloading_is_ignored() {
        // 进度事件仅对下载态生效(异常时序不得污染状态)
        let s = available();
        let s2 = apply_event(
            &s,
            UpdateEvent::DownloadProgress {
                downloaded_bytes: 10,
                total_bytes: 100,
            },
        );
        assert_eq!(s2, s);
    }

    #[test]
    fn dismissed_closes_card_from_closable_states() {
        // 卡片「稍后/关闭」:Available / Ready / Failed → Idle(壳页常驻后
        // 关闭卡片 = 状态归位,#36);下载中不可关闭,异常时序不得污染状态
        assert_eq!(apply_event(&available(), UpdateEvent::Dismissed), UpdateStateView::Idle);
        assert_eq!(
            apply_event(&UpdateStateView::Ready, UpdateEvent::Dismissed),
            UpdateStateView::Idle
        );
        assert_eq!(
            apply_event(
                &UpdateStateView::Failed {
                    error: UpdateError::DownloadFailed { detail: "x".into() }
                },
                UpdateEvent::Dismissed
            ),
            UpdateStateView::Idle
        );
        let downloading = apply_event(&available(), UpdateEvent::DownloadStarted { total_bytes: 0 });
        assert_eq!(apply_event(&downloading, UpdateEvent::Dismissed), downloading);
        assert_eq!(apply_event(&UpdateStateView::Idle, UpdateEvent::Dismissed), UpdateStateView::Idle);
    }

    #[test]
    fn recheck_from_available_and_failed_is_allowed() {
        // 卡片失败后可重试检查(6h 轮询/托盘手动);状态机层面一律允许,
        // 在途守卫在 UpdateManager::is_active(下载/就绪拒绝)
        for s in [available(), UpdateStateView::Failed {
            error: UpdateError::DownloadFailed { detail: "x".into() },
        }] {
            let s = apply_event(&s, UpdateEvent::CheckStarted);
            assert_eq!(s, UpdateStateView::Checking);
        }
    }

    #[test]
    fn update_state_view_serializes_with_status_tag() {
        // `update-state` 事件/`update_state` 命令的线上契约:内部 tag status,
        // 字段 camelCase;unit 变体无多余字段
        assert_eq!(
            serde_json::to_value(UpdateStateView::Idle).unwrap(),
            serde_json::json!({ "status": "idle" })
        );
        assert_eq!(
            serde_json::to_value(available()).unwrap(),
            serde_json::json!({
                "status": "available",
                "version": "0.2.0",
                "currentVersion": "0.1.0",
                "notes": null
            })
        );
        assert_eq!(
            serde_json::to_value(UpdateStateView::Ready).unwrap(),
            serde_json::json!({ "status": "ready" })
        );
        assert_eq!(
            serde_json::to_value(UpdateStateView::Failed {
                error: UpdateError::DownloadFailed { detail: "x".into() }
            })
            .unwrap(),
            serde_json::json!({
                "status": "failed",
                "error": { "kind": "DownloadFailed", "data": { "detail": "x" } }
            })
        );
    }
}
