//! 系统托盘:显示/隐藏窗口、退出。
//!
//! - 左键单击托盘图标:切换窗口显隐
//! - 右键菜单:显示/隐藏窗口、退出(退出先杀 dsh 子进程再 exit)

use tauri::menu::{MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::{dsh, locales};

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    // 托盘文案跟随系统语言(启动时检测一次;无语言设置页,启动检测即终局)
    let t = locales::shell_texts(locales::detect_lang());
    let toggle = MenuItem::with_id(app, "toggle", t.tray_toggle, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t.tray_quit, true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .item(&toggle)
        .separator()
        .item(&quit)
        .build()?;

    TrayIconBuilder::with_id("main-tray")
        .icon(tauri::include_image!("icons/32x32.png"))
        .menu(&menu)
        // 左键不弹菜单,留给"切换显隐";右键仍弹菜单
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_window(app),
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

fn toggle_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let visible = win.is_visible().unwrap_or(false);
    if visible {
        let _ = win.hide();
    } else {
        let _ = win.show();
        let _ = win.set_focus();
    }
}
