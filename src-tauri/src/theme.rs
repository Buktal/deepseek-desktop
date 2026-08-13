//! 外壳主题(亮/暗/跟随系统):单一事实源、解析、持久化与下发。
//!
//! #9 定稿:托盘主题子菜单三项勾选,勾选状态以 Rust 内存为事实源(菜单结构、
//! 勾选同步与菜单事件分发在 tray.rs)。本模块在其上落地 #8 的功能本体:
//!
//! - **持久化**:手动选择跨会话记忆,写 `<app_config_dir>/theme.json`
//!   (JSON `{"choice": "light"|"dark"|"system"}`,串即 `ThemeChoice::event_payload`
//!   的规范串);"跟随系统"是默认态,不写文件;读写失败只丢跨会话记忆,
//!   不阻塞主题切换。
//! - **解析**:选择 → 生效主题(light/dark);"跟随系统"读窗口 OS 主题
//!   (`Window::theme()`,tauri 2 theme API)。
//! - **下发**:Rust 为事实源,前端只消费生效主题——挂载时 invoke `theme_state`
//!   拉快照(监听先行、快照兜底,与 boot-state 同款竞态语义),此后监听
//!   `theme-changed` 事件(payload "light"|"dark");跟随系统时 OS 主题变化
//!   (`WindowEvent::ThemeChanged`,仅窗口主题为 None 时由 OS 触发)实时重推。
//! - **原生窗口**:`set_theme` 让标题栏与页面同主题;"跟随系统"置 None 跟随 OS。
//! - **事件契约(#9)**:托盘点击仍按 #9 契约推 `tray-theme`(选择串,见 tray.rs);
//!   本模块的 `theme-changed`(生效主题)是前端实际消费的事件——选择串
//!   "system" 对前端不可直接应用,解析在 Rust 侧单点完成,不重复 OS 检测。
//!
//! 全链映射单点维护:选择 ↔ 规范串 ↔ 菜单 id(`menu_id`) ↔ 生效主题(`resolve`),
//! 单测守住不变量,任一环漂移即红灯。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, Theme, WindowEvent};

/// 主题选择:托盘勾选状态与持久化的内存事实源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    Light,
    Dark,
    System,
}

impl ThemeChoice {
    /// 托盘菜单 id(#9 定稿;tray.rs 菜单构建与事件分发共用)。
    pub fn menu_id(self) -> &'static str {
        match self {
            ThemeChoice::Light => "theme-light",
            ThemeChoice::Dark => "theme-dark",
            ThemeChoice::System => "theme-system",
        }
    }

    /// 跨边界规范串:`tray-theme` 事件 payload(#9 契约)与配置文件共用,
    /// 选择 ↔ 串 的映射只此一份。
    pub fn event_payload(self) -> &'static str {
        match self {
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
            ThemeChoice::System => "system",
        }
    }

    /// 规范串 → 选择。未知串返回 None(调用方兜底,如配置文件损坏按默认处理)。
    pub fn from_payload(s: &str) -> Option<ThemeChoice> {
        match s {
            "light" => Some(ThemeChoice::Light),
            "dark" => Some(ThemeChoice::Dark),
            "system" => Some(ThemeChoice::System),
            _ => None,
        }
    }
}

/// 生效主题:选择 × OS 主题 → 实际亮/暗。纯函数,可测。
/// OS 主题不可得时按亮色(外壳无主题能力时即跟随系统,默认亮色系,与现状一致)。
fn resolve(choice: ThemeChoice, os: Option<Theme>) -> Theme {
    match choice {
        ThemeChoice::Light => Theme::Light,
        ThemeChoice::Dark => Theme::Dark,
        ThemeChoice::System => match os {
            Some(Theme::Dark) => Theme::Dark,
            _ => Theme::Light,
        },
    }
}

/// 当前选择(内存事实源;启动时由持久化初始化,默认"跟随系统")。
static THEME: Mutex<ThemeChoice> = Mutex::new(ThemeChoice::System);

pub fn current_choice() -> ThemeChoice {
    *THEME.lock().unwrap_or_else(|p| p.into_inner())
}

fn store_choice(choice: ThemeChoice) {
    if let Ok(mut g) = THEME.lock() {
        *g = choice;
    }
}

/// 配置文件:<app_config_dir>/theme.json(app_config_dir = config_dir/identifier,
/// Windows 即 %APPDATA%\app.deepseek-desktop)。
const CONFIG_FILE: &str = "theme.json";

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(CONFIG_FILE))
}

/// 读持久化选择。文件缺失/JSON 损坏/串未知 → None(默认"跟随系统")。
fn load_choice(path: &Path) -> Option<ThemeChoice> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    ThemeChoice::from_payload(value.get("choice")?.as_str()?)
}

/// 写持久化选择。目录缺失时创建;错误由调用方记录(失败只丢跨会话记忆)。
fn save_choice(path: &Path, choice: ThemeChoice) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&json!({ "choice": choice.event_payload() }))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, text)
}

/// 启动初始化(setup 中调用,须在 setup_tray 之前——托盘勾选读内存):
/// 读持久化 → 内存;窗口原生主题按选择应用;注册 OS 主题变化监听。
pub fn init(app: &AppHandle) {
    if let Some(path) = config_path(app) {
        if let Some(choice) = load_choice(&path) {
            store_choice(choice);
            log::info!("[theme] 从配置恢复主题选择: {:?}", choice);
        }
    }
    apply_native(app, current_choice());

    // OS 主题变化监听。WindowEvent::ThemeChanged 仅在窗口主题为 None
    // (跟随系统)时由 OS 变化触发(tauri WindowEvent 文档);显式亮/暗模式下
    // set_theme 引发的同名单事件由 System 守卫忽略(选择未变,无需重推)。
    if let Some(win) = app.get_webview_window("main") {
        let app = app.clone();
        win.on_window_event(move |event| {
            if matches!(event, WindowEvent::ThemeChanged(_))
                && current_choice() == ThemeChoice::System
            {
                log::info!("[theme] OS 主题变化 → 重推生效主题");
                push_resolved(&app);
            }
        });
    }
}

/// 选择变更(唯一写入口:托盘菜单点击经 tray.rs 收敛到此;将来的设置入口同此)。
/// 更新内存 → 持久化 → 原生窗口 → 下发。
pub fn choose(app: &AppHandle, choice: ThemeChoice) {
    store_choice(choice);
    match config_path(app) {
        Some(path) => {
            if let Err(e) = save_choice(&path, choice) {
                log::warn!("[theme] 主题选择持久化失败(仅影响跨会话记忆): {e}");
            }
        }
        None => log::warn!("[theme] 无法定位配置目录,主题选择不持久化"),
    }
    apply_native(app, choice);
    push_resolved(app);
}

/// 窗口原生主题(标题栏):亮/暗显式指定,"跟随系统"置 None 跟随 OS
/// (且只有 None 时 ThemeChanged 事件才会投递,恢复跟随语义的必要一步)。
fn apply_native(app: &AppHandle, choice: ThemeChoice) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let theme = match choice {
        ThemeChoice::Light => Some(Theme::Light),
        ThemeChoice::Dark => Some(Theme::Dark),
        ThemeChoice::System => None,
    };
    if let Err(e) = win.set_theme(theme) {
        log::warn!("[theme] 设置窗口主题失败: {e}");
    }
}

/// 当前生效主题:选择 × 窗口 OS 主题(tauri 2 theme API;窗口主题为 None
/// 时返回 OS 主题,与 resolve 的 System 分支互补)。
fn current_resolved(app: &AppHandle) -> Theme {
    let os = app.get_webview_window("main").and_then(|w| w.theme().ok());
    resolve(current_choice(), os)
}

/// 下发当前生效主题(`theme-changed`,payload "light"|"dark" 规范串)。
/// 与 theme_state 快照同源,前端"监听先行 + 快照兜底"。
fn push_resolved(app: &AppHandle) {
    let resolved = current_resolved(app);
    log::info!("[theme] 生效主题: {:?}", resolved);
    let _ = app.emit_to("main", "theme-changed", &resolved);
}

/// 生效主题快照命令:前端挂载时拉当前值(先注册监听再 invoke,
/// 与 boot-state 同款"后到者覆盖,来自同一状态"竞态语义)。
#[tauri::command]
pub async fn theme_state(app: tauri::AppHandle) -> Result<Theme, String> {
    Ok(current_resolved(&app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_payload_maps_canonical_strings() {
        assert_eq!(ThemeChoice::from_payload("light"), Some(ThemeChoice::Light));
        assert_eq!(ThemeChoice::from_payload("dark"), Some(ThemeChoice::Dark));
        assert_eq!(ThemeChoice::from_payload("system"), Some(ThemeChoice::System));
    }

    #[test]
    fn from_payload_rejects_unknown_strings() {
        assert_eq!(ThemeChoice::from_payload(""), None);
        assert_eq!(ThemeChoice::from_payload("Light"), None); // 规范串固定小写
        assert_eq!(ThemeChoice::from_payload("auto"), None);
    }

    #[test]
    fn resolve_maps_choice_and_os_theme() {
        // 显式选择与 OS 无关
        assert_eq!(resolve(ThemeChoice::Light, Some(Theme::Dark)), Theme::Light);
        assert_eq!(resolve(ThemeChoice::Dark, Some(Theme::Light)), Theme::Dark);
        // 跟随系统:取 OS 主题
        assert_eq!(resolve(ThemeChoice::System, Some(Theme::Dark)), Theme::Dark);
        assert_eq!(resolve(ThemeChoice::System, Some(Theme::Light)), Theme::Light);
        // OS 主题不可得:按亮色(默认态)
        assert_eq!(resolve(ThemeChoice::System, None), Theme::Light);
    }

    #[test]
    fn config_roundtrip_preserves_choice() {
        // 注意:每个测试用独立临时目录(并行执行互不踩踏,与 dsh.rs 测试同约定)
        let dir = std::env::temp_dir().join(format!(
            "dsh-theme-roundtrip-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(CONFIG_FILE);
        for choice in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
            save_choice(&path, choice).unwrap();
            assert_eq!(load_choice(&path), Some(choice));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_or_garbage_config_falls_back_to_none() {
        let dir = std::env::temp_dir().join(format!(
            "dsh-theme-garbage-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        assert_eq!(load_choice(&path), None); // 文件缺失
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_choice(&path), None); // JSON 损坏
        std::fs::write(&path, r#"{"choice":"neon"}"#).unwrap();
        assert_eq!(load_choice(&path), None); // 未知值
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_serializes_to_expected_shape() {
        // 配置文件与 event_payload 共用规范串:文件形状即契约,防序列化静默漂移
        let dir = std::env::temp_dir().join(format!(
            "dsh-theme-shape-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(CONFIG_FILE);
        save_choice(&path, ThemeChoice::System).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "{\n  \"choice\": \"system\"\n}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_choice_is_system() {
        // 无持久化时外壳即跟随系统;任何测试不得修改 THEME(互斥由串行执行保证)
        assert_eq!(current_choice(), ThemeChoice::System);
    }
}
