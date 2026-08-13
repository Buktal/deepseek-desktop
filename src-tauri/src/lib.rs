//! DeepSeek Desktop Tauri backend.
//!
//! 组装:dsh 生命周期管理 + 托盘 + 关闭三选对话框 + 退出收敛(杀子进程)+ 生产日志。

mod dsh;
mod logging;
mod tray;

use tauri::{Manager, WindowEvent};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例:防止双实例并发初始化 ~/.dsh profile
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 生产日志:落盘到 <app_data_dir>/logs/app.log + stdout,panic 经 hook 同落盘。
            // 失败不致命:降级为仅控制台输出,应用照常启动。
            if let Err(e) = logging::init(app.handle()) {
                eprintln!("[logging] init 失败,日志仅输出到控制台: {e}");
            }

            // dsh 管理器(Clone 共享内部 Arc 状态)
            let manager = dsh::DshManager::new(app.handle().clone());
            app.manage(manager.clone());

            // 托盘
            tray::setup_tray(app.handle())?;

            // 关闭按钮:原生三选对话框(退出应用/最小化到托盘/取消)
            setup_close_handler(app.handle(), manager.clone());

            // 立即启动 boot 流水线(窗口显示前就开始安装,前端挂载后拉快照)
            dsh::boot_start(&manager);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![dsh::boot, dsh::quit_app])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // 退出收敛:所有退出路径(托盘/对话框/quit_app)最终经 app.exit 汇入此处
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                if let Some(m) = app_handle.try_state::<dsh::DshManager>() {
                    dsh::kill_child(m.inner());
                }
            }
        });
}

/// 窗口 CloseRequested → 原生三选对话框。
/// 注意:CloseRequested 在 webview 与 window 层可能各触发一次,用 DIALOG_SHOWN 守卫防重复弹。
fn setup_close_handler(app: &tauri::AppHandle, manager: dsh::DshManager) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let app = app.clone();
    // receiver 与闭包捕获使用不同绑定,避免 move/借用冲突
    let handler_win = win.clone();
    win.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            // 程序化退出(托盘"退出"/quit_app)放行,不弹对话框
            if dsh::is_quitting() {
                return;
            }
            api.prevent_close();
            if !dsh::try_show_dialog() {
                return;
            }

            let win = handler_win.clone();
            let app = app.clone();
            let manager = manager.clone();
            app.dialog()
                .message("退出 DeepSeek Desktop?")
                .title("DeepSeek Desktop")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::YesNoCancelCustom(
                    "退出应用".into(),
                    "最小化到托盘".into(),
                    "取消".into(),
                ))
                .show_with_result(move |res| {
                    dsh::reset_dialog_flag();
                    match res {
                        MessageDialogResult::Custom(s) if s == "退出应用" => {
                            dsh::set_quitting();
                            dsh::kill_child(&manager);
                            app.exit(0);
                        }
                        MessageDialogResult::Custom(s) if s == "最小化到托盘" => {
                            let _ = win.hide();
                        }
                        _ => {} // 取消:保持现状
                    }
                });
        }
    });
}
