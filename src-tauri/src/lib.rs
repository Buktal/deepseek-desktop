//! DeepSeek Desktop Tauri backend.
//!
//! 组装:dsh 生命周期管理 + 托盘 + 关闭三选对话框 + 退出收敛(杀子进程)+ 生产日志。

mod dsh;
mod locales;
mod logging;
mod theme;
mod tray;

use tauri::{Manager, WindowEvent};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例:防止双实例并发初始化 ~/.dsh profile。第二个实例唤起主窗口:
        // unminimize 必须——最小化中的窗口只 show() 不会取消最小化(与 CC-Switch 同款处理)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
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

            // 主题:读持久化 → 内存 + 注册 OS 主题变化监听。
            // 须在 setup_tray 之前:托盘勾选状态读 theme::current_choice()
            theme::init(app.handle());

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
        .invoke_handler(tauri::generate_handler![
            dsh::boot,
            dsh::quit_app,
            theme::theme_state
        ])
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
/// 按钮顺序定稿(#9):[最小化到托盘, 退出应用, 取消]。rfd(Windows)用 TaskDialog 且
/// 不设默认按钮,默认按钮即第一个——把"最小化到托盘"放首位,Enter 只收起窗口、
/// 不退出应用(原顺序首按钮是"退出应用",回车直接杀进程,是误触隐患)。
fn setup_close_handler(app: &tauri::AppHandle, manager: dsh::DshManager) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let app = app.clone();
    // 关闭对话框文案跟随系统语言(启动时检测一次,与托盘同源)
    let t = locales::shell_texts(locales::detect_lang());
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
                .message(t.close_message)
                .title("DeepSeek Desktop")
                // Info:关闭确认不是错误/警告,图标用中性信息样式
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::YesNoCancelCustom(
                    t.close_minimize.into(),
                    t.close_quit.into(),
                    t.close_cancel.into(),
                ))
                .show_with_result(move |res| {
                    dsh::reset_dialog_flag();
                    match res {
                        MessageDialogResult::Custom(s) if s == t.close_quit => {
                            dsh::set_quitting();
                            dsh::kill_child(&manager);
                            app.exit(0);
                        }
                        MessageDialogResult::Custom(s) if s == t.close_minimize => {
                            let _ = win.hide();
                        }
                        _ => {} // 取消:保持现状
                    }
                });
        }
    });
}
