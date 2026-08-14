//! 开机自启(tauri-plugin-autostart):开关状态的单一事实源与托盘勾选渲染。
//!
//! - **事实源**:OS 启动项(Windows 即 HKCU `...\CurrentVersion\Run` 注册表值,
//!   由 auto-launch 读写)是自启状态的持久层与最终事实;本模块内存缓存只作
//!   托盘勾选的渲染事实源(#9 惯例:菜单勾选状态以 Rust 内存为事实源)。
//! - **启动恢复**:`init` 直查 OS 实际状态(`is_enabled`)初始化内存——不另写
//!   配置文件。双持久化(配置文件 + 注册表)会在用户用系统设置(如任务管理器
//!   的启动项页)修改自启时漂移,注册表是唯一持久层。
//! - **唯一写入口**:`set`。先调插件,成功才更新内存;失败保持内存 = OS 实际
//!   状态,托盘勾选不会显示从未真正生效的状态。
//! - **默认关闭**:OS 无启动项即关闭(插件初始状态),无需额外默认值。

use std::sync::Mutex;

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

/// 托盘菜单项 id(tray.rs 菜单构建与事件分发共用,单一事实源)。
pub const MENU_ID: &str = "autostart";

/// 当前自启状态(内存事实源;启动时由 OS 状态初始化,默认关闭)。
static ENABLED: Mutex<bool> = Mutex::new(false);

/// 托盘勾选渲染用:当前内存状态。
pub fn current() -> bool {
    *ENABLED.lock().unwrap_or_else(|p| p.into_inner())
}

fn store(enabled: bool) {
    if let Ok(mut g) = ENABLED.lock() {
        *g = enabled;
    }
}

/// 启动初始化(setup 中调用,须在 setup_tray 之前——托盘勾选读内存):
/// 直查 OS 启动项状态恢复内存。查询失败按关闭处理,应用照常启动。
pub fn init(app: &AppHandle) {
    let enabled = app.autolaunch().is_enabled().unwrap_or(false);
    store(enabled);
    log::info!("[autostart] 启动时自启状态: {enabled}");
}

/// 切换自启(唯一写入口:托盘点击收敛到此)。先调插件,成功才更新内存
/// (失败时内存保持 OS 实际状态,勾选不显示从未生效的状态)。
pub fn set(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let verb = if enabled { "开启" } else { "关闭" };
    let result = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    match result {
        Ok(()) => {
            store(enabled);
            log::info!("[autostart] 自启已{verb}");
            Ok(())
        }
        Err(e) => {
            log::warn!("[autostart] {verb}自启失败: {e}");
            Err(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_disabled() {
        // 自启默认关闭(#14 决策);测试不得修改 ENABLED(与 theme.rs 同约定)
        assert!(!current());
    }
}
