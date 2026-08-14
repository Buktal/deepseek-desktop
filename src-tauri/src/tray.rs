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
//!   检查逻辑全在 Rust 侧(update.rs),托盘点击直接调用 update 模块,前端不再监听;
//!   事件 emit 移除(不留死契约)。
//! - 升级通知形态(#3 §1,与 dsh 升级共用同一 Rust 侧机制):自动检测发现新版 →
//!   徽标图标变体 + 动态菜单项「升级到 vX」+ tooltip,不弹窗打断;点击动态菜单项
//!   → 显示窗口(若隐藏)→ 导航回本地升级页(update::navigate_to_shell)。
//! - 左键单击托盘图标:窗口可见且已聚焦时隐藏,否则显示并聚焦——纯 toggle 的陷阱是
//!   窗口被其它窗口挡住时,用户本想"唤出"结果却把窗口藏了。
//! - 退出:先杀 dsh 子进程再 exit(所有退出路径最终经 RunEvent::ExitRequested 再杀一次,
//!   kill_child 幂等,无副作用)。

use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::{autostart, dsh, locales, theme, update};
use crate::theme::ThemeChoice;

/// 托盘图标句柄(发现新版时换徽标变体 / 恢复,见 notify_update_available)。
static TRAY: Mutex<Option<TrayIcon<Wry>>> = Mutex::new(None);
/// 当前「升级到 vX」菜单项标签(Some = 发现新版,菜单重建时插入动态项;
/// 单一事实源,notify_update_available/notify_clear_update 维护)。
static UPGRADE_LABEL: Mutex<Option<String>> = Mutex::new(None);

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

/// 构建托盘菜单(可复用:发现新版时重建并插入动态「升级到 vX」项)。
/// upgrade_label = Some(版本标签)时在「检查更新」上方插入 id="upgrade-available" 项。
fn build_menu(app: &AppHandle, upgrade_label: Option<String>) -> tauri::Result<Menu<Wry>> {
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
    if let Some(label) = upgrade_label {
        let upgrade = MenuItem::with_id(app, "upgrade-available", label, true, None::<&str>)?;
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

/// 按当前状态重建菜单(UPGRADE_LABEL + 主题内存),应用到托盘。
/// 主题点击后也要重建:Windows 勾选菜单项不会自动互斥,重建让三个勾选
/// 回到内存事实源(theme.rs),避免视觉漂移。
fn refresh_menu(app: &AppHandle) {
    let label = UPGRADE_LABEL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let menu = build_menu(app, label);
    if let Some(tray) = TRAY.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tray.set_menu(menu.ok());
    }
}

/// 发现新版:换徽标图标变体 + 重建菜单(动态「升级到 vX」项)+ tooltip 提示。
/// 由 update.rs 检查结果调用(#3 §1 通知形态)。
pub fn notify_update_available(app: &AppHandle, version: &str) {
    let t = locales::shell_texts(locales::detect_lang());
    let label = t.tray_upgrade_label(version);
    if let Ok(mut g) = UPGRADE_LABEL.lock() {
        *g = Some(label.clone());
    }
    refresh_menu(app);
    if let Some(tray) = TRAY.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tray.set_icon(Some(badge_icon()));
        let _ = tray.set_tooltip(Some(t.tray_tooltip_available(version)));
    }
    log::info!("[tray] 发现新版 → 徽标 + 菜单项「{label}」");
}

/// 无新版/检查失败:恢复普通图标与菜单(动态项移除)。
pub fn notify_clear_update(app: &AppHandle) {
    if let Ok(mut g) = UPGRADE_LABEL.lock() {
        *g = None;
    }
    refresh_menu(app);
    if let Some(tray) = TRAY.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tray.set_icon(Some(normal_icon()));
        let _ = tray.set_tooltip(Some("DeepSeek Desktop".to_string()));
    }
    log::info!("[tray] 恢复普通托盘图标");
}

fn normal_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/32x32.png")
}

/// 徽标变体:右上角圆点(形状表达,不依赖颜色,深浅任务栏均可见——#3 §1)。
fn badge_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/32x32-badge.png")
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, None)?;
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(normal_icon())
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
            "upgrade-available" => {
                // 被动通知入口(#3 §1):显示窗口(若隐藏)→ 导航回本地升级页
                log::info!("[tray] 菜单[升级] → 导航升级卡片");
                update::navigate_to_shell(app);
            }
            "check-update" => {
                // #3 事件契约变更:检查逻辑全在 Rust 侧,托盘点击直接触发
                // 手动检查(检查结束弹原生对话框直接回答),不再推前端事件
                log::info!("[tray] 检查更新(手动)");
                if let Some(m) = app.try_state::<update::UpdateManager>() {
                    m.inner().check_now(true);
                }
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
