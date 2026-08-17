//! 生产日志基建。
//!
//! - 常规日志(boot 流水线各阶段/错误/dsh 输出)经 `log` crate 宏 → tauri-plugin-log,
//!   落盘到系统临时目录 `<temp>/deepseek-desktop/logs/app.log`(5MB 轮转,KeepOne),
//!   同时输出到 stdout。
//! - panic hook:把 panic 消息 + 位置 + backtrace 也走 log::error 写入同一日志文件,
//!   发布后无需复现即可从日志文件定位崩溃点。
//!
//! 初始化失败不致命:调用方降级为仅控制台输出,应用照常启动。

use std::fs;

/// 初始化日志插件与 panic hook。须在 setup 中、任何业务线程启动前调用一次。
pub fn init(app: &tauri::AppHandle) -> Result<(), String> {
    // 日志是临时诊断信息:放系统 Temp 而非应用数据目录,不随应用数据持久化、易清理
    let log_dir = std::env::temp_dir().join("deepseek-desktop").join("logs");
    fs::create_dir_all(&log_dir)
        .map_err(|e| format!("无法创建日志目录 {}: {e}", log_dir.display()))?;

    app.plugin(
        tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .targets([
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                    path: log_dir,
                    file_name: Some("app".into()),
                }),
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
            ])
            .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
            .max_file_size(5 * 1024 * 1024)
            .build(),
    )
    .map_err(|e| format!("日志插件注册失败: {e}"))?;

    setup_panic_hook();
    Ok(())
}

/// 设置 panic hook:panic 消息 + 位置 + backtrace 经 log::error 落盘。
/// 保留默认 hook(stderr 输出),便于开发期直接看到 panic。
fn setup_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            format!("{info}")
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".into());
        let backtrace = std::backtrace::Backtrace::force_capture();
        // log crate 内部不会 panic,panic hook 里调用是安全的
        log::error!("panic: {msg}\n    at {location}\n{backtrace}");
        default_hook(info);
    }));
}
