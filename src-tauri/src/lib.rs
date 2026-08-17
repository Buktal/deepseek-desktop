//! DeepSeek Desktop Tauri backend.
//!
//! 组装:dsh 生命周期管理 + 应用自身升级 + 托盘 + 关闭三选对话框 + 退出收敛(杀子进程)
//! + 生产日志 + 窗口状态记忆 + 开机自启。

mod autostart;
mod close;
mod dsh;
mod error;
mod locales;
mod logging;
mod menu;
mod navigation;
mod npm;
mod proc;
mod theme;
mod tray;
mod update;
mod upgrade;

use tauri::{LogicalSize, Manager, PhysicalSize, WindowEvent};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

use crate::close::CloseBehavior;

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
        // 应用自身升级:check/download/install 全在 Rust(update.rs,照搬
        // O_CC_One 的插件组合——updater 检查下载 + process/opener 配套能力)
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        // 窗口状态记忆:大小/位置/最大化跨会话恢复(tauri-plugin-window-state)。
        // 只持久化 POSITION|SIZE|MAXIMIZED,排除 VISIBLE——窗口从托盘隐藏/
        // 最小化后退出,重启仍正常显示窗口(CC-Switch 同款 flags)。
        // 保存时机:RunEvent::Exit 自动落盘(我们的退出路径是正常 app.exit(0),
        // 不绕过 run loop,无需像 CC-Switch 那样手动 save_window_state)。
        // 恢复时机:窗口 ready 时自动应用;保存位置不在当前任一显示器内时由 OS
        // 决定放置(防拔显示器后窗口飞出屏外);SIZE 恢复受窗口最小尺寸约束钳制。
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        // 开机自启(Windows:注册表 Run 键;默认关闭,托盘菜单开关项,#14)
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            // 生产日志:落盘到 <temp>/deepseek-desktop/logs/app.log + stdout,panic 经 hook 同落盘。
            // 失败不致命:降级为仅控制台输出,应用照常启动。
            if let Err(e) = logging::init(app.handle()) {
                eprintln!("[logging] init 失败,日志仅输出到控制台: {e}");
            }

            // 主题:读持久化 → 内存。须在 setup_tray 之前(托盘勾选状态读
            // theme::current_choice());原生主题应用与 OS 主题监听在窗口创建
            // 后经 attach_main_window 完成——主窗口 create: false,由
            // navigation.rs 在 setup 中 builder 创建,此时尚不存在(#37 修
            // #15 引入的时序回归)
            theme::init(app.handle());

            // 开机自启:直查 OS 启动项状态 → 内存(默认关闭)。
            // 须在 setup_tray 之前:托盘勾选状态读 autostart::current()
            autostart::init(app.handle());

            // 关闭行为:读持久化 → 内存(默认"每次询问")。
            // 须在 setup_tray 之前:托盘勾选状态读 close::current();close
            // handler 在窗口事件中读(close.rs,#38)
            close::init(app.handle());

            // dsh 管理器(Clone 共享内部 Arc 状态)。先 manage:导航拦截
            // 回调经 try_state 读 dsh URL(#15),顺序无硬依赖,放这里语义就近
            let manager = dsh::DshManager::new(app.handle().clone());
            app.manage(manager.clone());

            // 主窗口:config `create: false`(#15),此处用 builder 创建并挂
            // 导航拦截 + 页面层外链拦截脚本(见 navigation.rs:on_navigation /
            // on_new_window 只存在于 builder;iframe 外链经注入脚本 postMessage
            // 回壳页,opener 开系统浏览器)
            navigation::create_main_window(app.handle())?;

            // 主题:窗口创建后应用原生主题(启动恢复持久化选择)+ 注册 OS
            // 主题变化监听(init 时窗口尚不存在,见上;#37)
            theme::attach_main_window(app.handle());

            // 窗口恢复后几何钳制:restore 的保存值可能小于 minWidth/minHeight
            // (插件 set_size 是编程 resize,OS 不强制 min 约束),此处保证实际
            // 尺寸不小于 config 的最小值。双保险:同步检查 + Resized 监听,
            // 覆盖插件 restore 在 setup 前后两种完成时序。
            enforce_min_window_size(app.handle());

            // 应用升级管理器 + 常驻检查(启动探测 + 6h 轮询,#9:检查逻辑在 Rust 侧)
            let updater = update::UpdateManager::new(app.handle().clone());
            app.manage(updater.clone());
            updater.start_resident_checks();

            // dsh 升级管理器 + 常驻检查(启动探测 + 6h 轮询,与应用升级共用
            // 触发时机;托盘手动入口组合编排在 tray::on_check_update,#17)
            let dsh_upgrade = upgrade::UpgradeManager::new(app.handle().clone());
            app.manage(dsh_upgrade.clone());
            dsh_upgrade.start_resident_checks();

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
            theme::theme_state,
            update::update_state,
            update::update_apply,
            update::update_restart,
            update::update_dismiss,
            upgrade::upgrade_state,
            upgrade::upgrade_confirm,
            upgrade::upgrade_dismiss,
            tray::menu_snapshot,
            tray::menu_action
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
            // 关闭行为(#38):非 Ask 直接执行;Ask 期间暂维持原生三选弹窗
            // (M4 换 UI,见 #31)。行为的内存事实源在 close.rs,菜单勾选同源。
            match close::current() {
                CloseBehavior::Quit => {
                    dsh::set_quitting();
                    dsh::kill_child(&manager);
                    app.exit(0);
                }
                CloseBehavior::Minimize => {
                    let _ = handler_win.hide();
                }
                CloseBehavior::Ask => {
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
            }
        }
    });
}

/// 窗口恢复后的几何钳制:保证实际尺寸不小于 tauri.conf.json 的 minWidth/minHeight
/// (最小尺寸的单一事实源是 config,此处只做运行时强制)。
///
/// 背景:OS 编程 resize(set_size/MoveWindow)不强制 min 约束——只有用户交互拖动
/// 受 track size 限制,故窗口状态插件 restore 保存的小尺寸(如旧显示器/DPI 环境
/// 下保存的值)时需主动钳制,否则窗口可能小于最小尺寸。
///
/// 双保险:setup 同步检查(插件 restore 若在 setup 前完成即生效)+ Resized 监听
/// (restore 在 setup 后完成、以及 DPI 变化等后续路径)。clamp 后 Resized 再触发
/// 时条件已不满足,无循环。
fn enforce_min_window_size(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let Some(cfg) = app.config().app.windows.iter().find(|w| w.label == "main") else {
        return;
    };
    let min_w = cfg.min_width.unwrap_or(0.0);
    let min_h = cfg.min_height.unwrap_or(0.0);
    if min_w <= 0.0 && min_h <= 0.0 {
        return;
    }
    clamp_window(&win, min_w, min_h);
    let handler_win = win.clone();
    win.on_window_event(move |event| {
        if let WindowEvent::Resized(_) = event {
            clamp_window(&handler_win, min_w, min_h);
        }
    });
}

/// 当前物理尺寸换算逻辑后小于 min 时 set_size 到 min。
fn clamp_window(win: &tauri::WebviewWindow, min_w: f64, min_h: f64) {
    let Ok(size) = win.inner_size() else {
        return;
    };
    let scale = win.scale_factor().unwrap_or(1.0);
    if let Some((w, h)) = clamp_size(size, scale, (min_w, min_h)) {
        let _ = win.set_size(LogicalSize::new(w, h));
    }
}

/// 纯函数:物理尺寸 × 缩放 → 逻辑尺寸,任一维度低于 min 时返回钳制后的逻辑尺寸。
fn clamp_size(size: PhysicalSize<u32>, scale: f64, min: (f64, f64)) -> Option<(f64, f64)> {
    let w = size.width as f64 / scale;
    let h = size.height as f64 / scale;
    let (cw, ch) = (w.max(min.0), h.max(min.1));
    (cw != w || ch != h).then_some((cw, ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_size_noop_within_min() {
        // 尺寸 ≥ min 时不干预
        assert_eq!(clamp_size(PhysicalSize::new(1000, 800), 1.0, (840.0, 600.0)), None);
        assert_eq!(clamp_size(PhysicalSize::new(840, 600), 1.0, (840.0, 600.0)), None);
    }

    #[test]
    fn clamp_size_enforces_min() {
        // 单维度低于 min 只钳该维度,另一维度保持
        assert_eq!(
            clamp_size(PhysicalSize::new(500, 300), 1.0, (840.0, 600.0)),
            Some((840.0, 600.0))
        );
        assert_eq!(
            clamp_size(PhysicalSize::new(500, 900), 1.0, (840.0, 600.0)),
            Some((840.0, 900.0))
        );
        assert_eq!(
            clamp_size(PhysicalSize::new(1000, 300), 1.0, (840.0, 600.0)),
            Some((1000.0, 600.0))
        );
    }

    #[test]
    fn clamp_size_uses_logical_units() {
        // 200% DPI:物理 1000x300 = 逻辑 500x150 < min → 钳到逻辑 840x600
        assert_eq!(
            clamp_size(PhysicalSize::new(1000, 300), 2.0, (840.0, 600.0)),
            Some((840.0, 600.0))
        );
        // 200% DPI:物理 1900x1200 = 逻辑 950x600 ≥ min → 不干预
        assert_eq!(
            clamp_size(PhysicalSize::new(1900, 1200), 2.0, (840.0, 600.0)),
            None
        );
    }
}
