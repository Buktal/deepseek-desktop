//! 关闭行为(Ask|Minimize|Quit):单一事实源、解析、持久化与下发。
//!
//! #38 施工(拍板 #33「close_behavior 设置」):菜单「设置▸关闭行为」三选,
//! 勾选状态以本模块内存为事实源(菜单快照构建在 menu.rs,动作分发在 tray.rs);
//! close handler 读之——非 Ask 直接执行(退出应用 / 最小化到托盘),
//! Ask 期间暂维持原生三选弹窗(M4 换 UI,见 #31 规格)。
//!
//! 持久化与 theme.rs 同模式:
//! - 写 `<app_config_dir>/close.json`(JSON `{"choice": "ask"|"minimize"|"quit"}`,
//!   串即 `CloseBehavior::payload` 的规范串);默认态 Ask 不写文件;
//!   读写失败只丢跨会话记忆,不阻塞行为生效。
//! - 启动时 `init` 读持久化进内存(须在 setup_tray 之前——托盘勾选读内存)。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::json;
use tauri::{AppHandle, Manager};

/// 关闭行为:点窗口关闭按钮时的去向(单一事实源)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseBehavior {
    /// 每次询问(现状:原生三选弹窗,M4 换 UI)
    Ask,
    /// 最小化到托盘
    Minimize,
    /// 退出应用
    Quit,
}

impl CloseBehavior {
    /// 菜单项 id(menu.rs 快照构建与 tray.rs 动作分发共用,单一事实来源)。
    pub fn menu_id(self) -> &'static str {
        match self {
            CloseBehavior::Ask => "close-ask",
            CloseBehavior::Minimize => "close-minimize",
            CloseBehavior::Quit => "close-quit",
        }
    }

    /// 跨边界规范串:配置文件与前端契约共用,选择 ↔ 串 的映射只此一份。
    pub fn payload(self) -> &'static str {
        match self {
            CloseBehavior::Ask => "ask",
            CloseBehavior::Minimize => "minimize",
            CloseBehavior::Quit => "quit",
        }
    }

    /// 规范串 → 选择。未知串返回 None(调用方兜底,如配置文件损坏按默认处理)。
    pub fn from_payload(s: &str) -> Option<CloseBehavior> {
        match s {
            "ask" => Some(CloseBehavior::Ask),
            "minimize" => Some(CloseBehavior::Minimize),
            "quit" => Some(CloseBehavior::Quit),
            _ => None,
        }
    }
}

/// 当前行为(内存事实源;启动时由持久化初始化,默认"每次询问")。
static BEHAVIOR: Mutex<CloseBehavior> = Mutex::new(CloseBehavior::Ask);

pub fn current() -> CloseBehavior {
    *BEHAVIOR.lock().unwrap_or_else(|p| p.into_inner())
}

fn store(behavior: CloseBehavior) {
    if let Ok(mut g) = BEHAVIOR.lock() {
        *g = behavior;
    }
}

/// 配置文件:<app_config_dir>/close.json(app_config_dir = config_dir/identifier,
/// Windows 即 %APPDATA%\app.deepseek-desktop,与 theme.json 同目录)。
const CONFIG_FILE: &str = "close.json";

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(CONFIG_FILE))
}

/// 读持久化行为。文件缺失/JSON 损坏/串未知 → None(默认"每次询问")。
fn load_behavior(path: &Path) -> Option<CloseBehavior> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    CloseBehavior::from_payload(value.get("choice")?.as_str()?)
}

/// 写持久化行为。目录缺失时创建;错误由调用方记录(失败只丢跨会话记忆)。
fn save_behavior(path: &Path, behavior: CloseBehavior) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&json!({ "choice": behavior.payload() }))
        .map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// 启动初始化(setup 中调用,须在 setup_tray 之前——托盘勾选读内存):
/// 读持久化 → 内存。
pub fn init(app: &AppHandle) {
    if let Some(path) = config_path(app) {
        if let Some(behavior) = load_behavior(&path) {
            store(behavior);
            log::info!("[close] 从配置恢复关闭行为: {:?}", behavior);
        }
    }
}

/// 行为变更(唯一写入口:菜单「设置▸关闭行为」点击经 tray.rs 分发收敛到此;
/// 将来的设置入口同此)。更新内存 → 持久化;菜单勾选由调用方 refresh 重建。
pub fn set(app: &AppHandle, behavior: CloseBehavior) {
    store(behavior);
    match config_path(app) {
        Some(path) => {
            if let Err(e) = save_behavior(&path, behavior) {
                log::warn!("[close] 关闭行为持久化失败(仅影响跨会话记忆): {e}");
            }
        }
        None => log::warn!("[close] 无法定位配置目录,关闭行为不持久化"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_payload_maps_canonical_strings() {
        assert_eq!(CloseBehavior::from_payload("ask"), Some(CloseBehavior::Ask));
        assert_eq!(
            CloseBehavior::from_payload("minimize"),
            Some(CloseBehavior::Minimize)
        );
        assert_eq!(CloseBehavior::from_payload("quit"), Some(CloseBehavior::Quit));
    }

    #[test]
    fn from_payload_rejects_unknown_strings() {
        assert_eq!(CloseBehavior::from_payload(""), None);
        assert_eq!(CloseBehavior::from_payload("Ask"), None); // 规范串固定小写
        assert_eq!(CloseBehavior::from_payload("auto"), None);
    }

    #[test]
    fn menu_id_roundtrips_through_payload() {
        // 不变量:菜单 id ↔ 选择 ↔ 规范串 一一对应,互不漂移
        // (映射的单一事实源在 close.rs,这里守住菜单侧的不变量)
        for behavior in [
            CloseBehavior::Ask,
            CloseBehavior::Minimize,
            CloseBehavior::Quit,
        ] {
            assert_eq!(
                CloseBehavior::from_payload(behavior.payload()),
                Some(behavior)
            );
            assert!(!behavior.menu_id().is_empty());
            assert!(!behavior.payload().is_empty());
        }
        // payload 是前端契约:固定小写英文串
        assert_eq!(CloseBehavior::Ask.payload(), "ask");
        assert_eq!(CloseBehavior::Minimize.payload(), "minimize");
        assert_eq!(CloseBehavior::Quit.payload(), "quit");
    }

    #[test]
    fn config_roundtrip_preserves_behavior() {
        // 注意:每个测试用独立临时目录(并行执行互不踩踏,与 theme.rs 同约定)
        let dir = std::env::temp_dir().join(format!(
            "dsh-close-roundtrip-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(CONFIG_FILE);
        for behavior in [CloseBehavior::Ask, CloseBehavior::Minimize, CloseBehavior::Quit] {
            save_behavior(&path, behavior).unwrap();
            assert_eq!(load_behavior(&path), Some(behavior));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_or_garbage_config_falls_back_to_none() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-close-garbage-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        assert_eq!(load_behavior(&path), None); // 文件缺失
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_behavior(&path), None); // JSON 损坏
        std::fs::write(&path, r#"{"choice":"always-ask"}"#).unwrap();
        assert_eq!(load_behavior(&path), None); // 未知值
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_serializes_to_expected_shape() {
        // 配置文件与 payload 共用规范串:文件形状即契约,防序列化静默漂移
        let dir = std::env::temp_dir().join(format!(
            "dsh-close-shape-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(CONFIG_FILE);
        save_behavior(&path, CloseBehavior::Quit).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "{\n  \"choice\": \"quit\"\n}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_behavior_is_ask() {
        // 无持久化时即现状(每次询问);任何测试不得修改 BEHAVIOR(互斥由串行执行保证)
        assert_eq!(current(), CloseBehavior::Ask);
    }
}
