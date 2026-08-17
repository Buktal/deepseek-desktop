//! 系统托盘(定稿结构,见 #9):显示/隐藏窗口、主题、开机自启、检查更新、退出。
//!
//! 菜单结构(#9 定稿 + #3 动态项 + #14 自启开关):
//! ```text
//! 显示/隐藏窗口   id="toggle"
//! ───────────────
//! 主题 ▸         id="theme"(子菜单)
//!   亮色         id="theme-light"   (勾选)
//!   暗色         id="theme-dark"    (勾选)
//!   跟随系统     id="theme-system"  (勾选,默认)
//! 开机自启       id="autostart"     (勾选,默认关,#14)
//! ───────────────
//! 升级到 vX      id="upgrade-available"(仅发现新版时存在,动态,#3 §1)
//! 检查更新       id="check-update"
//! ───────────────
//! 退出           id="quit"
//! ```
//! 图标:原生托盘菜单不支持图标(tauri 菜单项无图标参数,Windows 托盘菜单本就不渲染
//! 图标),文案即全部表达。
//!
//! 功能分发:
//! - 主题:点击主题项 → `theme::choose`(theme.rs 是主题的单一事实源:
//!   更新内存、持久化、同步原生窗口、推 `theme-changed` 生效主题事件给 boot UI)。
//!   勾选状态以 theme.rs 内存为事实源;本处仍按 #9 契约推 `tray-theme` 事件
//!   (payload 为 "light"|"dark"|"system" 选择串),boot UI 实际消费的是
//!   `theme-changed`("light"|"dark" 生效主题,见 theme.rs 模块文档)。
//!   注意:事件只到 boot UI(dsh 页是 remote origin,ACL 拒绝,见 dsh.rs 安全语义),
//!   与红线一致——dsh 页面不碰主题。
//! - 检查更新(#3 事件契约变更):原占位「推 `tray-check-update` 事件给前端」被取代——
//!   检查逻辑全在 Rust 侧(update.rs / upgrade.rs),托盘点击直接调用检查模块,
//!   前端不再监听;事件 emit 移除(不留死契约)。
//! - 升级通知形态(#3 §1,两层升级共用同一 Rust 侧机制):自动检测发现新版 →
//!   徽标图标变体 + 动态菜单项(app「升级到 vX」/ dsh「升级 dsh 到 vX」)+ tooltip,
//!   不弹窗打断;点击动态菜单项 → 显示窗口 + 推卡片请求事件(upgrade-card-request
//!   / update-card-request),前端按状态渲染对应升级卡片浮层(壳页常驻,无整窗
//!   导航,#36;自动检测只亮徽标不弹卡片,#3 §1)。
//! - 手动检查入口(#17 组合编排 on_check_update):dsh 层先答(dsh 新版 → dsh
//!   对话框;检查失败 → 失败对话框),应用层兜底(应用新版 → 应用对话框;无新版
//!   → 合并「已是最新」对话框附 dsh 版本);dsh 升级流水线在途时 no-op(#3 边界)。
//! - 左键单击托盘图标:窗口可见且已聚焦时隐藏,否则显示并聚焦——纯 toggle 的陷阱是
//!   窗口被其它窗口挡住时,用户本想"唤出"结果却把窗口藏了。
//! - 退出:先杀 dsh 子进程再 exit(所有退出路径最终经 RunEvent::ExitRequested 再杀一次,
//!   kill_child 幂等,无副作用)。

use std::sync::Mutex;
use std::thread;

use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::{autostart, dsh, locales, theme, update, upgrade};
use crate::theme::ThemeChoice;

/// 托盘图标句柄(发现新版时换徽标变体 / 恢复,见 set_app_update/set_dsh_update)。
static TRAY: Mutex<Option<TrayIcon<Wry>>> = Mutex::new(None);
/// 升级通知槽位(单一事实源,set_app_update/set_dsh_update 维护):
/// 存「待升级版本」(非标签文案——语言在 build_menu 时才判定),
/// 任一非空 → 徽标图标变体;动态菜单项按优先级插入(先 dsh 后应用)。
static APP_UPDATE_VERSION: Mutex<Option<String>> = Mutex::new(None);
static DSH_UPDATE_VERSION: Mutex<Option<String>> = Mutex::new(None);

/// 菜单事件 id → 主题选择。纯函数,可测;未知 id 返回 None。
/// 主题状态与映射的单一事实源在 theme.rs(ThemeChoice::menu_id ↔ from_payload)。
fn theme_choice_from_id(id: &str) -> Option<ThemeChoice> {
    match id {
        "theme-light" => Some(ThemeChoice::Light),
        "theme-dark" => Some(ThemeChoice::Dark),
        "theme-system" => Some(ThemeChoice::System),
        _ => None,
    }
}

/// 构建托盘菜单(可复用:发现新版时重建并插入动态升级项)。
/// app_version / dsh_version = 待升级版本(槽位,语言在此判定拼标签):
/// - dsh 新版 → id="upgrade-dsh"「升级 dsh 到 vX」(优先级高,排最前)
/// - 应用新版 → id="upgrade-available"「升级到 vX」
fn build_menu(
    app: &AppHandle,
    app_version: Option<&str>,
    dsh_version: Option<&str>,
) -> tauri::Result<Menu<Wry>> {
    let t = locales::shell_texts(locales::detect_lang());
    let toggle = MenuItem::with_id(app, "toggle", t.tray_toggle, true, None::<&str>)?;

    // 主题三项:勾选状态以 theme::current_choice() 为单一事实来源;
    // 菜单 id 以 ThemeChoice::menu_id() 为单一来源(重建时勾选状态随内存走)
    let theme_light = CheckMenuItem::with_id(
        app,
        ThemeChoice::Light.menu_id(),
        t.tray_theme_light,
        true,
        theme::current_choice() == ThemeChoice::Light,
        None::<&str>,
    )?;
    let theme_dark = CheckMenuItem::with_id(
        app,
        ThemeChoice::Dark.menu_id(),
        t.tray_theme_dark,
        true,
        theme::current_choice() == ThemeChoice::Dark,
        None::<&str>,
    )?;
    let theme_system = CheckMenuItem::with_id(
        app,
        ThemeChoice::System.menu_id(),
        t.tray_theme_system,
        true,
        theme::current_choice() == ThemeChoice::System,
        None::<&str>,
    )?;
    let theme = SubmenuBuilder::with_id(app, "theme", t.tray_theme)
        .item(&theme_light)
        .item(&theme_dark)
        .item(&theme_system)
        .build()?;

    // 开机自启:勾选状态以 autostart::current() 为单一事实来源
    // (内存由 autostart::init 从 OS 启动项恢复;重建时勾选随内存走)
    let autostart_item = CheckMenuItem::with_id(
        app,
        autostart::MENU_ID,
        t.tray_autostart,
        true,
        autostart::current(),
        None::<&str>,
    )?;

    let mut builder = MenuBuilder::new(app)
        .item(&toggle)
        .separator()
        .item(&theme)
        .item(&autostart_item)
        .separator();
    // 动态升级项:先 dsh 后应用(任一存在即插入对应项,不存在则不占位)
    if let Some(v) = dsh_version {
        let upgrade = MenuItem::with_id(
            app,
            "upgrade-dsh",
            t.tray_upgrade_dsh_label(v),
            true,
            None::<&str>,
        )?;
        builder = builder.item(&upgrade);
    }
    if let Some(v) = app_version {
        let upgrade = MenuItem::with_id(app, "upgrade-available", t.tray_upgrade_label(v), true, None::<&str>)?;
        builder = builder.item(&upgrade);
    }
    let check_update =
        MenuItem::with_id(app, "check-update", t.tray_check_update, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t.tray_quit, true, None::<&str>)?;
    builder
        .item(&check_update)
        .separator()
        .item(&quit)
        .build()
}

/// 按当前状态重建菜单与徽标(槽位 + 主题内存),应用到托盘。
/// 主题点击后也要重建:Windows 勾选菜单项不会自动互斥,重建让三个勾选
/// 回到内存事实源(theme.rs),避免视觉漂移。
fn refresh_menu(app: &AppHandle) {
    let (app_version, dsh_version) = update_slots();
    let menu = build_menu(app, app_version.as_deref(), dsh_version.as_deref());
    if let Some(tray) = TRAY.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tray.set_menu(menu.ok());
    }
    // 徽标 + tooltip:任一槽位非空即徽标变体;tooltip 优先 dsh(主产品)
    let t = locales::shell_texts(locales::detect_lang());
    let badge = app_version.is_some() || dsh_version.is_some();
    let tooltip = match dsh_version.as_deref() {
        Some(v) => t.tray_tooltip_dsh_available(v),
        None => app_version
            .as_deref()
            .map(|v| t.tray_tooltip_available(v))
            .unwrap_or_else(|| "DeepSeek Desktop".to_string()),
    };
    if let Some(tray) = TRAY.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tray.set_icon(Some(if badge { badge_icon() } else { normal_icon() }));
        // macOS:菜单栏图标按 template 渲染(黑+透明,深浅菜单栏自动适配);
        // set_icon 后须同步 template 状态(两方法均跨平台可调用,内部按平台生效)
        let _ = tray.set_icon_as_template(cfg!(target_os = "macos"));
        let _ = tray.set_tooltip(Some(tooltip));
    }
}

fn update_slots() -> (Option<String>, Option<String>) {
    let app_version = APP_UPDATE_VERSION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let dsh_version = DSH_UPDATE_VERSION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    (app_version, dsh_version)
}

/// 设置应用自身升级通知槽位(Some = 发现新版,None = 清除;由 update.rs 调用)。
/// 徽标/菜单/tooltip 在 refresh_menu 统一呈现(#3 §1 通知形态)。
pub fn set_app_update(app: &AppHandle, version: Option<&str>) {
    if let Ok(mut g) = APP_UPDATE_VERSION.lock() {
        *g = version.map(String::from);
    }
    log::info!("[tray] 应用升级槽位 → {version:?}");
    refresh_menu(app);
}

/// 设置 dsh 升级通知槽位(Some = 发现新版,None = 清除;由 upgrade.rs 调用)。
/// 徽标/菜单/tooltip 在 refresh_menu 统一呈现(#3 §1 通知形态)。
pub fn set_dsh_update(app: &AppHandle, version: Option<&str>) {
    if let Ok(mut g) = DSH_UPDATE_VERSION.lock() {
        *g = version.map(String::from);
    }
    log::info!("[tray] dsh 升级槽位 → {version:?}");
    refresh_menu(app);
}

fn normal_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/32x32.png")
}

/// 徽标变体:右上角圆点(形状表达,不依赖颜色,深浅任务栏均可见——#3 §1)。
fn badge_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/32x32-badge.png")
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, None, None)?;
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(normal_icon())
        // macOS 菜单栏惯例:图标按 template 渲染(黑白自适应深浅菜单栏)
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&menu)
        // 左键不弹菜单,留给"切换显隐";右键仍弹菜单
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_window(app),
            id if id.starts_with("theme-") => {
                if let Some(choice) = theme_choice_from_id(id) {
                    // 主题勾选状态由 build_menu 在重建时按内存事实源恢复,
                    // 此处只负责状态变更与事件下发
                    on_theme_chosen(app, choice);
                }
            }
            autostart::MENU_ID => on_autostart_toggled(app),
            "upgrade-dsh" | "upgrade-available" => {
                // 被动通知入口(#3 §1,两层共用):显示窗口 + 推卡片请求事件,
                // 前端按状态渲染对应卡片浮层(壳页常驻,无整窗导航,#36;
                // available 态需此显式请求才弹卡片,自动检测只亮徽标)
                let card = if event.id().as_ref() == "upgrade-dsh" {
                    "upgrade-card-request"
                } else {
                    "update-card-request"
                };
                log::info!("[tray] 菜单[升级] → 显示窗口 + 推卡片请求 {card}");
                show_main_window(app);
                let _ = app.emit_to("main", card, ());
            }
            "check-update" => {
                // #3 事件契约变更 + #17 组合编排:检查逻辑全在 Rust 侧,
                // 两层升级共用托盘手动入口,直接回答(见 on_check_update)
                log::info!("[tray] 检查更新(手动)");
                on_check_update(app);
            }
            "quit" => {
                dsh::set_quitting();
                if let Some(m) = app.try_state::<dsh::DshManager>() {
                    dsh::kill_child(m.inner());
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;
    if let Ok(mut g) = TRAY.lock() {
        *g = Some(tray);
    }
    Ok(())
}

/// 主题菜单点击分发:状态变更收敛到 theme::choose(单一事实源,负责持久化与
/// `theme-changed` 下发);本处按 #9 契约推 `tray-theme` 选择串事件,并重建
/// 菜单让三个勾选回到内存事实源(Windows 勾选项不自动互斥,见 refresh_menu)。
fn on_theme_chosen(app: &AppHandle, choice: ThemeChoice) {
    theme::choose(app, choice);
    refresh_menu(app);
    log::info!("[tray] 主题切换: {choice:?} → 推 tray-theme 事件");
    let _ = app.emit_to("main", "tray-theme", choice.event_payload());
}

/// 自启菜单点击分发:切换收敛到 autostart::set(唯一写入口,插件失败时内存
/// 保持 OS 实际状态);重建菜单让勾选回到内存事实源(Windows 勾选项不自动
/// 切换,与主题同款处理,见 refresh_menu)。
fn on_autostart_toggled(app: &AppHandle) {
    let next = !autostart::current();
    log::info!("[tray] 切换开机自启 → {next}");
    let _ = autostart::set(app, next);
    refresh_menu(app);
}

/// 托盘「检查更新」手动入口的组合编排(#3 §1「直接回答」+ #17 两层共用触发):
///
/// 1. dsh 升级流水线在途 → no-op(#3 边界:UPGRADING 守卫,菜单点击仍可看进度);
/// 2. dsh 层先答(`upgrade::manual_check`):dsh 新版 → dsh 对话框 [升级][稍后]
///    (boot 未就绪时只亮徽标);检查失败 → 「检查更新失败,请稍后重试」;
///    已用对话框回答 → 结束,不再弹应用层对话框(避免叠加);
/// 3. 应用层兜底(`update::check_now` 结果回调):应用新版 → 应用对话框;
///    无新版 → 合并「已是最新」对话框(附 dsh 版本,一次回答两层);
///    应用检查失败 → 沿用现状静默。
fn on_check_update(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        if let Some(up) = app.try_state::<upgrade::UpgradeManager>() {
            if up.inner().is_pipeline_running() {
                log::info!("[tray] dsh 升级流水线在途,手动检查 no-op(#3 边界)");
                return;
            }
        }
        // 1. dsh 层(同步检查,3-5s 超时;回答过即结束)
        let dsh_outcome = upgrade::manual_check(&app);
        if dsh_outcome.answered {
            return;
        }
        // 2. 应用层(异步,结果经回调做对话框决策)
        if let Some(m) = app.try_state::<update::UpdateManager>() {
            let app2 = app.clone();
            let dsh_version = dsh_outcome.installed_version;
            m.inner().check_now(true, Some(Box::new(move |r| match r {
                update::ManualCheckResult::Found { version, current_version } => {
                    update::show_update_found_dialog(&app2, &version, &current_version);
                }
                update::ManualCheckResult::None => {
                    update::show_up_to_date_dialog(&app2, dsh_version.as_deref());
                }
                update::ManualCheckResult::Failed => {
                    // 应用侧检查失败沿用现状:静默(等下一次触发)
                    log::warn!("[tray] 应用更新检查失败(静默)");
                }
            })));
        }
    });
}

/// 显示并聚焦主窗口(取消最小化)。托盘动态升级菜单项与手动检查对话框
/// [升级] 共用——壳页常驻后不再整窗导航,「看升级卡片」= 显示窗口 +
/// 推卡片请求事件(或流水线状态自动弹卡)。
pub(crate) fn show_main_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

/// 显示/隐藏窗口。行为:窗口可见且已聚焦时隐藏,否则显示并聚焦。
/// 左键单击与菜单项共用同一语义。
fn toggle_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let visible = win.is_visible().unwrap_or(false);
    if visible && win.is_focused().unwrap_or(false) {
        let _ = win.hide();
    } else {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_choice_from_id_maps_all_menu_ids() {
        assert_eq!(theme_choice_from_id("theme-light"), Some(ThemeChoice::Light));
        assert_eq!(theme_choice_from_id("theme-dark"), Some(ThemeChoice::Dark));
        assert_eq!(theme_choice_from_id("theme-system"), Some(ThemeChoice::System));
        assert_eq!(theme_choice_from_id("theme-foo"), None);
        assert_eq!(theme_choice_from_id("toggle"), None);
    }

    #[test]
    fn theme_menu_id_and_payload_roundtrip() {
        // 不变量:菜单 id ↔ 选择 ↔ 前端事件 payload 一一对应,互不漂移
        // (映射的单一事实源在 theme.rs,这里守住托盘侧的不变量)
        for choice in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
            assert_eq!(theme_choice_from_id(choice.menu_id()), Some(choice));
            assert!(!choice.event_payload().is_empty());
        }
        // payload 是前端契约:固定小写英文串
        assert_eq!(ThemeChoice::Light.event_payload(), "light");
        assert_eq!(ThemeChoice::Dark.event_payload(), "dark");
        assert_eq!(ThemeChoice::System.event_payload(), "system");
    }
}
