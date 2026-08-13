//! 系统托盘(定稿结构,见 #9):显示/隐藏窗口、主题、检查更新、退出。
//!
//! 菜单结构(#9 定稿):
//! ```text
//! 显示/隐藏窗口   id="toggle"
//! ───────────────
//! 主题 ▸         id="theme"(子菜单)
//!   亮色         id="theme-light"   (勾选)
//!   暗色         id="theme-dark"    (勾选)
//!   跟随系统     id="theme-system"  (勾选,默认)
//! ───────────────
//! 检查更新       id="check-update"
//! ───────────────
//! 退出           id="quit"
//! ```
//! 图标:原生托盘菜单不支持图标(tauri 菜单项无图标参数,Windows 托盘菜单本就不渲染
//! 图标),文案即全部表达。
//!
//! 主题三项与检查更新是 #8/#5 的功能入口:本文件定结构、文案与菜单分发,
//! 功能本体由对应 ticket 落地——
//! - #8 主题切换:点击主题项 → `theme::choose`(theme.rs 是主题的单一事实源:
//!   更新内存、持久化、同步原生窗口、推 `theme-changed` 生效主题事件给 boot UI)。
//!   勾选状态以 theme.rs 内存为事实源;本处仍按 #9 契约推 `tray-theme` 事件
//!   (payload 为 "light"|"dark"|"system" 选择串),boot UI 实际消费的是
//!   `theme-changed`("light"|"dark" 生效主题,见 theme.rs 模块文档)。
//!   注意:事件只到 boot UI(dsh 页是 remote origin,ACL 拒绝,见 dsh.rs 安全语义),
//!   与红线一致——dsh 页面不碰主题。
//! - #5 应用自身升级:前端监听 `tray-check-update` 事件(无 payload)。
//! - 左键单击托盘图标:窗口可见且已聚焦时隐藏,否则显示并聚焦——纯 toggle 的陷阱是
//!   窗口被其它窗口挡住时,用户本想"唤出"结果却把窗口藏了。
//! - 退出:先杀 dsh 子进程再 exit(所有退出路径最终经 RunEvent::ExitRequested 再杀一次,
//!   kill_child 幂等,无副作用)。

use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::{dsh, locales, theme};
use crate::theme::ThemeChoice;

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

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    // 托盘文案跟随系统语言(启动时检测一次;无语言设置页,启动检测即终局)
    let t = locales::shell_texts(locales::detect_lang());
    let toggle = MenuItem::with_id(app, "toggle", t.tray_toggle, true, None::<&str>)?;

    // 主题三项:勾选状态以 theme::current_choice() 为单一事实来源(#8 落地后
    // 由持久化值初始化,见 theme.rs);菜单 id 以 ThemeChoice::menu_id() 为单一来源
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

    let check_update =
        MenuItem::with_id(app, "check-update", t.tray_check_update, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t.tray_quit, true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .item(&toggle)
        .separator()
        .item(&theme)
        .separator()
        .item(&check_update)
        .separator()
        .item(&quit)
        .build()?;

    TrayIconBuilder::with_id("main-tray")
        .icon(tauri::include_image!("icons/32x32.png"))
        .menu(&menu)
        // 左键不弹菜单,留给"切换显隐";右键仍弹菜单
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "toggle" => toggle_window(app),
            id if id.starts_with("theme-") => {
                if let Some(choice) = theme_choice_from_id(id) {
                    on_theme_chosen(app, choice, &theme_light, &theme_dark, &theme_system);
                }
            }
            "check-update" => {
                // #5 挂接点:推给前端 update-check 逻辑(仅 boot UI 能收事件,
                // dsh 页为 remote origin 被 ACL 拒绝,见 dsh.rs 安全语义)
                log::info!("[tray] 检查更新: 推 tray-check-update 事件给前端");
                let _ = app.emit_to("main", "tray-check-update", ());
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
    Ok(())
}

/// 主题菜单点击分发:状态变更收敛到 theme::choose(单一事实源,负责持久化与
/// `theme-changed` 下发);本处只做托盘 UI 自己的事——同步三个勾选 +
/// 按 #9 契约推 `tray-theme` 选择串事件。
fn on_theme_chosen(
    app: &AppHandle,
    choice: ThemeChoice,
    light: &CheckMenuItem<Wry>,
    dark: &CheckMenuItem<Wry>,
    system: &CheckMenuItem<Wry>,
) {
    theme::choose(app, choice);
    let _ = light.set_checked(choice == ThemeChoice::Light);
    let _ = dark.set_checked(choice == ThemeChoice::Dark);
    let _ = system.set_checked(choice == ThemeChoice::System);
    log::info!("[tray] 主题切换: {choice:?} → 推 tray-theme 事件");
    let _ = app.emit_to("main", "tray-theme", choice.event_payload());
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
