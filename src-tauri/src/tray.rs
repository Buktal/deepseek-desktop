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
//! 主题三项与检查更新是 #8/#5 的功能占位:本文件定结构、文案与分发逻辑
//! (菜单点击 → 事件推前端),功能本体由对应 ticket 挂接——
//! - #8 主题切换:前端监听 `tray-theme` 事件(payload 为 "light"|"dark"|"system"
//!   字符串),boot UI 应用主题;勾选状态以本模块内存为事实源。
//!   注意:事件只到 boot UI(dsh 页是 remote origin,ACL 拒绝,见 dsh.rs 安全语义),
//!   与红线一致——dsh 页面不碰主题。
//! - #5 应用自身升级:前端监听 `tray-check-update` 事件(无 payload)。
//! - 左键单击托盘图标:窗口可见且已聚焦时隐藏,否则显示并聚焦——纯 toggle 的陷阱是
//!   窗口被其它窗口挡住时,用户本想"唤出"结果却把窗口藏了。
//! - 退出:先杀 dsh 子进程再 exit(所有退出路径最终经 RunEvent::ExitRequested 再杀一次,
//!   kill_child 幂等,无副作用)。

use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::{dsh, locales};

/// 主题选择:托盘勾选状态的内存事实源(#8 持久化后由持久化值初始化)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    Light,
    Dark,
    System,
}

impl ThemeChoice {
    fn menu_id(self) -> &'static str {
        match self {
            ThemeChoice::Light => "theme-light",
            ThemeChoice::Dark => "theme-dark",
            ThemeChoice::System => "theme-system",
        }
    }

    /// 推给前端的事件 payload("light"|"dark"|"system")。
    fn event_payload(self) -> &'static str {
        match self {
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
            ThemeChoice::System => "system",
        }
    }
}

/// 菜单事件 id → 主题选择。纯函数,可测;未知 id 返回 None。
fn theme_choice_from_id(id: &str) -> Option<ThemeChoice> {
    match id {
        "theme-light" => Some(ThemeChoice::Light),
        "theme-dark" => Some(ThemeChoice::Dark),
        "theme-system" => Some(ThemeChoice::System),
        _ => None,
    }
}

/// 当前主题选择。默认"跟随系统"(与现状一致:外壳无主题能力,即跟随系统)。
static THEME: Mutex<ThemeChoice> = Mutex::new(ThemeChoice::System);

fn current_theme() -> ThemeChoice {
    *THEME.lock().unwrap_or_else(|p| p.into_inner())
}

fn set_theme(choice: ThemeChoice) {
    if let Ok(mut g) = THEME.lock() {
        *g = choice;
    }
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    // 托盘文案跟随系统语言(启动时检测一次;无语言设置页,启动检测即终局)
    let t = locales::shell_texts(locales::detect_lang());
    let toggle = MenuItem::with_id(app, "toggle", t.tray_toggle, true, None::<&str>)?;

    // 主题三项:勾选状态以 current_theme() 为单一事实来源;
    // 菜单 id 以 ThemeChoice::menu_id() 为单一来源(事件分发经 theme_choice_from_id 回映)
    let theme_light = CheckMenuItem::with_id(
        app,
        ThemeChoice::Light.menu_id(),
        t.tray_theme_light,
        true,
        current_theme() == ThemeChoice::Light,
        None::<&str>,
    )?;
    let theme_dark = CheckMenuItem::with_id(
        app,
        ThemeChoice::Dark.menu_id(),
        t.tray_theme_dark,
        true,
        current_theme() == ThemeChoice::Dark,
        None::<&str>,
    )?;
    let theme_system = CheckMenuItem::with_id(
        app,
        ThemeChoice::System.menu_id(),
        t.tray_theme_system,
        true,
        current_theme() == ThemeChoice::System,
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

/// 主题菜单点击分发:更新内存选择 + 同步三个勾选 + 推 `tray-theme` 事件给前端。
/// #8 在此挂接持久化与应用;当前点击已有真实勾选反馈(勾选状态即结构的一部分)。
fn on_theme_chosen(
    app: &AppHandle,
    choice: ThemeChoice,
    light: &CheckMenuItem<Wry>,
    dark: &CheckMenuItem<Wry>,
    system: &CheckMenuItem<Wry>,
) {
    set_theme(choice);
    let _ = light.set_checked(choice == ThemeChoice::Light);
    let _ = dark.set_checked(choice == ThemeChoice::Dark);
    let _ = system.set_checked(choice == ThemeChoice::System);
    log::info!("[tray] 主题切换: {:?} → 推 tray-theme 事件", choice);
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
        for choice in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
            assert_eq!(theme_choice_from_id(choice.menu_id()), Some(choice));
            assert!(!choice.event_payload().is_empty());
        }
        // payload 是前端契约:固定小写英文串
        assert_eq!(ThemeChoice::Light.event_payload(), "light");
        assert_eq!(ThemeChoice::Dark.event_payload(), "dark");
        assert_eq!(ThemeChoice::System.event_payload(), "system");
    }

    #[test]
    fn default_theme_is_system() {
        // 无主题能力时外壳即跟随系统;任何测试不得修改 THEME(与本测试互斥由串行执行保证)
        assert_eq!(current_theme(), ThemeChoice::System);
    }
}
