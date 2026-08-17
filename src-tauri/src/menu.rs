//! 菜单快照(拍板 #33 / 施工 #38):平台无关菜单模型的唯一构建处。
//!
//! 单一事实来源 = Rust 的平台无关菜单模型,本模块产出 MenuSnapshot(纯函数:
//! 状态源 + locale → 快照);两个薄投影层消费同一快照,前端零菜单逻辑:
//! - muda 投影(tray.rs `build_menu`):托盘右键菜单
//! - emit 投影(tray.rs `refresh_menu` → `menu-state` 事件 + `menu_snapshot`
//!   命令):壳菜单条 shadcn DropdownMenu 纯映射渲染
//!
//! 状态源(theme.rs / autostart.rs / tray.rs 升级槽位 / upgrade.rs 流水线 /
//! close.rs)是事实源,快照只是投影;每次状态变化 refresh_menu 重建托盘 +
//! emit 新快照,勾选互斥靠重建(Windows 原生勾选不自动互斥)。
//!
//! 文案由 locales::ShellTexts 解析后放进快照(动态升级项标签也在此插值),
//! 前端不放第二份文案表——zh/en 仅 Rust locales 一处。
//!
//! 菜单结构(#9 定稿 + #14 自启 + #3 动态升级项 + #38 设置▸关闭行为):
//! ```text
//! 显示/隐藏窗口   id="toggle"
//! ───────────────
//! 主题 ▸         id="theme"(子菜单)
//!   亮色         id="theme-light"   (勾选)
//!   暗色         id="theme-dark"    (勾选)
//!   跟随系统     id="theme-system"  (勾选,默认)
//! 设置 ▸         id="settings"(子菜单,#38)
//!   关闭行为 ▸   id="close-behavior"(子菜单)
//!     每次询问   id="close-ask"      (勾选,默认)
//!     最小化到托盘 id="close-minimize"(勾选)
//!     退出应用   id="close-quit"     (勾选)
//! 开机自启       id="autostart"     (勾选,默认关,#14)
//! ───────────────
//! 升级到 vX      id="upgrade-available"(仅发现新版时存在,动态,#3 §1,badge)
//! 升级 dsh 到 vX id="upgrade-dsh"    (仅发现新版时存在,动态,#3 §1,badge;
//!                                       dsh 升级流水线在途时 disabled,#40)
//! 检查更新       id="check-update"   (任一升级流水线在途时 disabled)
//! ───────────────
//! 退出           id="quit"
//! ```

use serde::Serialize;

use crate::close::CloseBehavior;
use crate::locales::ShellTexts;
use crate::theme::ThemeChoice;

/// 菜单项类型(序列化小写串,前端按 kind 分发渲染)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MenuItemKind {
    /// 点击即动作
    Action,
    /// 勾选态(勾选由快照承载,点击后经 menu_action 回流,新快照覆盖)
    Check,
    /// 分隔线(无 id/label 语义)
    Separator,
    /// 子菜单(children 承载)
    Submenu,
}

/// 菜单项(快照的最小单元)。checked/disabled/badge/children 均为可选字段,
/// 序列化时缺省不出现(前端按 kind 分发,缺失字段即默认值)。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub kind: MenuItemKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    /// 升级徽标(原型疑点 2 的机制预留):值 = 待升级版本;前端菜单按钮
    /// 徽标点据此显示,与托盘徽标图标变体同源(#3 §1 通知形态)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<MenuItem>>,
}

/// 菜单快照:`menu-state` 事件与 `menu_snapshot` 命令的载荷。
/// button_label:壳菜单条按钮文案(Rust locales 解析,前端不持有第二份文案)。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MenuSnapshot {
    pub button_label: String,
    pub items: Vec<MenuItem>,
}

/// 快照构建所需的状态输入(收集自各事实源,tray.rs `collect_menu_state`)。
/// 纯值结构:与 AppHandle 无关,快照构建可在测试中直接调用。
#[derive(Debug, Clone)]
pub struct MenuState {
    pub theme_choice: ThemeChoice,
    pub autostart: bool,
    /// 应用升级槽位(Some = 发现新版,待升级版本)
    pub app_update: Option<String>,
    /// dsh 升级槽位(Some = 发现新版,待升级版本)
    pub dsh_update: Option<String>,
    /// 任一升级流水线在途(dsh 升级 Active 或应用升级下载/就绪;#39「检查
    /// 更新」disabled 的依据,与 tray 动作层的 no-op 守卫同源——行为先有
    /// 守卫,快照只是让 UI 先于点击诚实呈现)
    pub upgrade_running: bool,
    /// dsh 升级流水线在途(「升级 dsh」动态条目 disabled 的依据,#40;
    /// 与 upgrade_running 分离:两层升级独立,应用流水线不置灰 dsh 条目)
    pub dsh_upgrade_running: bool,
    pub close_behavior: CloseBehavior,
}

impl MenuState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        theme_choice: ThemeChoice,
        autostart: bool,
        app_update: Option<String>,
        dsh_update: Option<String>,
        upgrade_running: bool,
        dsh_upgrade_running: bool,
        close_behavior: CloseBehavior,
    ) -> Self {
        Self {
            theme_choice,
            autostart,
            app_update,
            dsh_update,
            upgrade_running,
            dsh_upgrade_running,
            close_behavior,
        }
    }
}

/// 动作项。disabled/badge 缺省不序列化。
fn action(id: &str, label: String) -> MenuItem {
    MenuItem {
        id: id.into(),
        label,
        kind: MenuItemKind::Action,
        checked: None,
        disabled: None,
        badge: None,
        children: None,
    }
}

fn check(id: &str, label: &str, checked: bool) -> MenuItem {
    MenuItem {
        id: id.into(),
        label: label.into(),
        kind: MenuItemKind::Check,
        checked: Some(checked),
        disabled: None,
        badge: None,
        children: None,
    }
}

fn submenu(id: &str, label: String, children: Vec<MenuItem>) -> MenuItem {
    MenuItem {
        id: id.into(),
        label,
        kind: MenuItemKind::Submenu,
        checked: None,
        disabled: None,
        badge: None,
        children: Some(children),
    }
}

fn separator() -> MenuItem {
    MenuItem {
        id: String::new(),
        label: String::new(),
        kind: MenuItemKind::Separator,
        checked: None,
        disabled: None,
        badge: None,
        children: None,
    }
}

/// 快照构建:状态源 + locale → 平台无关菜单快照。纯函数,可测;
/// 生产路径唯一调用方是 tray.rs(收集状态 → 构建 → 两个投影)。
pub fn build_snapshot(t: &ShellTexts, s: &MenuState) -> MenuSnapshot {
    // 主题三项:勾选状态以 theme::current_choice() 为事实源,菜单 id 以
    // ThemeChoice::menu_id() 为单一来源(theme.rs)
    let theme_children = [
        ThemeChoice::Light,
        ThemeChoice::Dark,
        ThemeChoice::System,
    ]
    .map(|choice| {
        check(
            choice.menu_id(),
            match choice {
                ThemeChoice::Light => t.tray_theme_light,
                ThemeChoice::Dark => t.tray_theme_dark,
                ThemeChoice::System => t.tray_theme_system,
            },
            s.theme_choice == choice,
        )
    })
    .to_vec();

    // 设置 ▸ 关闭行为:勾选状态以 close::current() 为事实源(#38)
    let close_children = [
        CloseBehavior::Ask,
        CloseBehavior::Minimize,
        CloseBehavior::Quit,
    ]
    .map(|behavior| {
        check(
            behavior.menu_id(),
            match behavior {
                CloseBehavior::Ask => t.close_ask,
                // 与关闭三选弹窗按钮同文案(locales 一处,#38)
                CloseBehavior::Minimize => t.close_minimize,
                CloseBehavior::Quit => t.close_quit,
            },
            s.close_behavior == behavior,
        )
    })
    .to_vec();

    let mut items = vec![
        action("toggle", t.tray_toggle.into()),
        separator(),
        submenu("theme", t.tray_theme.into(), theme_children),
        submenu(
            "settings",
            t.tray_settings.into(),
            vec![submenu("close-behavior", t.tray_close_behavior.into(), close_children)],
        ),
        check(autostart_id(), t.tray_autostart, s.autostart),
        separator(),
    ];
    // 动态升级项:先 dsh 后应用(任一存在即插入对应项,不存在则不占位,#3 §1);
    // badge 承载待升级版本(前端菜单按钮徽标点)
    if let Some(v) = &s.dsh_update {
        let mut item = upgrade_item("upgrade-dsh", t.tray_upgrade_dsh_label(v), v);
        // #40:升级流水线在途时「升级 dsh」置灰(快照机制;升级会重杀 dsh,
        // 与「检查更新」的 disabled 同款「UI 先于点击诚实呈现」——点击侧本
        // 就无意义:覆盖层已在升级中,再弹请求无效果)
        if s.dsh_upgrade_running {
            item.disabled = Some(true);
        }
        items.push(item);
    }
    if let Some(v) = &s.app_update {
        items.push(upgrade_item("upgrade-available", t.tray_upgrade_label(v), v));
    }
    let mut check_update = action("check-update", t.tray_check_update.into());
    if s.upgrade_running {
        check_update.disabled = Some(true);
    }
    items.push(check_update);
    items.push(separator());
    items.push(action("quit", t.tray_quit.into()));

    MenuSnapshot {
        button_label: t.menu_button.into(),
        items,
    }
}

fn upgrade_item(id: &str, label: String, version: &str) -> MenuItem {
    let mut item = action(id, label);
    item.badge = Some(version.into());
    item
}

/// 开机自启菜单项 id(autostart.rs 的 MENU_ID 是动作分发的事实源,
/// 快照构建复用同一常量,防两份 id 漂移)。
fn autostart_id() -> &'static str {
    crate::autostart::MENU_ID
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locales::{self, Lang};

    fn zh() -> ShellTexts {
        locales::shell_texts(Lang::Zh)
    }

    fn en() -> ShellTexts {
        locales::shell_texts(Lang::En)
    }

    /// 默认状态:跟随系统 / 自启关 / 无升级 / 流水线空闲 / 每次询问。
    fn default_state() -> MenuState {
        MenuState::new(
            ThemeChoice::System,
            false,
            None,
            None,
            false,
            false,
            CloseBehavior::Ask,
        )
    }

    #[test]
    fn default_state_renders_full_structure() {
        // 菜单结构定稿(#9 + #14 + #3 + #38):顶层序列逐项对齐
        let snap = build_snapshot(&zh(), &default_state());
        assert_eq!(snap.button_label, "菜单");
        let ids: Vec<&str> = snap.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            ["toggle", "", "theme", "settings", "autostart", "", "check-update", "", "quit"]
        );
        // 类型序列:action / separator / submenu / check 交替正确
        assert_eq!(snap.items[0].kind, MenuItemKind::Action);
        assert_eq!(snap.items[1].kind, MenuItemKind::Separator);
        assert_eq!(snap.items[2].kind, MenuItemKind::Submenu);
        assert_eq!(snap.items[3].kind, MenuItemKind::Submenu);
        assert_eq!(snap.items[4].kind, MenuItemKind::Check);
        assert_eq!(snap.items[6].kind, MenuItemKind::Action);
        // 默认勾选:跟随系统 + 每次询问;自启与流水线默认 off
        assert_eq!(snap.items[4].checked, Some(false));
        assert_eq!(snap.items[6].disabled, None);
    }

    #[test]
    fn theme_submenu_checks_follow_state() {
        // 勾选互斥由快照保证:同一子菜单内恰一项 checked
        for (choice, expected_id) in [
            (ThemeChoice::Light, "theme-light"),
            (ThemeChoice::Dark, "theme-dark"),
            (ThemeChoice::System, "theme-system"),
        ] {
            let s = MenuState {
                theme_choice: choice,
                ..default_state()
            };
            let snap = build_snapshot(&zh(), &s);
            let theme = snap.items.iter().find(|i| i.id == "theme").unwrap();
            let children = theme.children.as_ref().unwrap();
            let checked: Vec<&str> = children
                .iter()
                .filter(|c| c.checked == Some(true))
                .map(|c| c.id.as_str())
                .collect();
            assert_eq!(checked, [expected_id]);
        }
    }

    #[test]
    fn close_behavior_submenu_checks_follow_state() {
        // 「设置▸关闭行为」三选:勾选互斥,勾选项随 close::current()
        for (behavior, expected_id) in [
            (CloseBehavior::Ask, "close-ask"),
            (CloseBehavior::Minimize, "close-minimize"),
            (CloseBehavior::Quit, "close-quit"),
        ] {
            let s = MenuState {
                close_behavior: behavior,
                ..default_state()
            };
            let snap = build_snapshot(&zh(), &s);
            let settings = snap.items.iter().find(|i| i.id == "settings").unwrap();
            let close_behavior = settings
                .children
                .as_ref()
                .unwrap()
                .iter()
                .find(|c| c.id == "close-behavior")
                .unwrap();
            let checked: Vec<&str> = close_behavior
                .children
                .as_ref()
                .unwrap()
                .iter()
                .filter(|c| c.checked == Some(true))
                .map(|c| c.id.as_str())
                .collect();
            assert_eq!(checked, [expected_id]);
        }
    }

    #[test]
    fn autostart_check_follows_state() {
        let s = MenuState {
            autostart: true,
            ..default_state()
        };
        let snap = build_snapshot(&zh(), &s);
        let autostart = snap.items.iter().find(|i| i.id == autostart_id()).unwrap();
        assert_eq!(autostart.checked, Some(true));
    }

    #[test]
    fn upgrade_items_appear_dynamically_with_badge() {
        // 动态升级项:先 dsh 后应用;badge 承载待升级版本(#3 §1 通知形态)
        let s = MenuState {
            app_update: Some("0.5.0".into()),
            dsh_update: Some("0.1.0-rc.9".into()),
            ..default_state()
        };
        let snap = build_snapshot(&zh(), &s);
        let ids: Vec<&str> = snap.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            ["toggle", "", "theme", "settings", "autostart", "", "upgrade-dsh", "upgrade-available", "check-update", "", "quit"]
        );
        let dsh_item = snap.items.iter().find(|i| i.id == "upgrade-dsh").unwrap();
        assert_eq!(dsh_item.badge.as_deref(), Some("0.1.0-rc.9"));
        assert_eq!(dsh_item.label, "升级 dsh 到 v0.1.0-rc.9");
        let app_item = snap.items.iter().find(|i| i.id == "upgrade-available").unwrap();
        assert_eq!(app_item.badge.as_deref(), Some("0.5.0"));
        assert_eq!(app_item.label, "升级到 v0.5.0");
        // 无升级槽位:动态项不占位
        let snap = build_snapshot(&zh(), &default_state());
        assert!(snap.items.iter().all(|i| i.id != "upgrade-dsh" && i.id != "upgrade-available"));
    }

    #[test]
    fn dsh_upgrade_pipeline_disables_upgrade_dsh_item() {
        // #40:「升级 dsh」动态条目在 dsh 升级流水线在途时置灰(快照机制);
        // 应用流水线在途不置灰(两层升级独立)
        let s = MenuState {
            dsh_update: Some("0.1.0-rc.9".into()),
            dsh_upgrade_running: true,
            ..default_state()
        };
        let snap = build_snapshot(&zh(), &s);
        let item = snap.items.iter().find(|i| i.id == "upgrade-dsh").unwrap();
        assert_eq!(item.disabled, Some(true));
        // badge 保留(通知形态与置灰不冲突)
        assert_eq!(item.badge.as_deref(), Some("0.1.0-rc.9"));
        // 应用流水线在途:dsh 条目保持可点
        let s = MenuState {
            dsh_update: Some("0.1.0-rc.9".into()),
            upgrade_running: true, // 应用升级下载/就绪
            dsh_upgrade_running: false,
            ..default_state()
        };
        let snap = build_snapshot(&zh(), &s);
        let item = snap.items.iter().find(|i| i.id == "upgrade-dsh").unwrap();
        assert_eq!(item.disabled, None);
        // 流水线结束后恢复可点
        let s = MenuState {
            dsh_update: Some("0.1.0-rc.9".into()),
            ..default_state()
        };
        let snap = build_snapshot(&zh(), &s);
        let item = snap.items.iter().find(|i| i.id == "upgrade-dsh").unwrap();
        assert_eq!(item.disabled, None);
    }

    #[test]
    fn upgrade_pipeline_disables_check_update() {
        // 「升级中 disabled」:流水线在途 → check-update disabled
        // (tray 动作层本有 no-op 守卫,快照让 UI 先于点击诚实呈现,#38 验收)
        let s = MenuState {
            upgrade_running: true,
            ..default_state()
        };
        let snap = build_snapshot(&zh(), &s);
        let check_update = snap.items.iter().find(|i| i.id == "check-update").unwrap();
        assert_eq!(check_update.disabled, Some(true));
        // 其余动作项不受影响
        let quit = snap.items.iter().find(|i| i.id == "quit").unwrap();
        assert_eq!(quit.disabled, None);
        // 流水线结束后恢复可点
        let snap = build_snapshot(&zh(), &default_state());
        let check_update = snap.items.iter().find(|i| i.id == "check-update").unwrap();
        assert_eq!(check_update.disabled, None);
    }

    #[test]
    fn labels_follow_locale() {
        // 文案 zh/en 仅 Rust locales 一处:快照是解析后的投影
        let snap_zh = build_snapshot(&zh(), &default_state());
        let snap_en = build_snapshot(&en(), &default_state());
        assert_eq!(snap_zh.button_label, "菜单");
        assert_eq!(snap_en.button_label, "Menu");
        assert_eq!(snap_zh.items[0].label, "显示/隐藏窗口");
        assert_eq!(snap_en.items[0].label, "Show/Hide window");
        let settings_zh = snap_zh.items.iter().find(|i| i.id == "settings").unwrap();
        let settings_en = snap_en.items.iter().find(|i| i.id == "settings").unwrap();
        assert_eq!(settings_zh.label, "设置");
        assert_eq!(settings_en.label, "Settings");
    }

    #[test]
    fn snapshot_serializes_to_expected_shape() {
        // 线上契约(menu-state 事件 / menu_snapshot 命令):camelCase,
        // 可选字段缺省不出现;kind 小写串
        let snap = build_snapshot(&zh(), &default_state());
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["buttonLabel"], "菜单");
        let first = &v["items"][0];
        assert_eq!(first["id"], "toggle");
        assert_eq!(first["kind"], "action");
        assert!(first.get("checked").is_none());
        assert!(first.get("disabled").is_none());
        let theme = v["items"].as_array().unwrap().iter().find(|i| i["id"] == "theme").unwrap();
        assert_eq!(theme["kind"], "submenu");
        // 勾选项显式序列化 checked(默认跟随系统:light=false, system=true)
        assert_eq!(theme["children"][0]["checked"], false);
        assert_eq!(theme["children"][2]["checked"], true);
        let check_update = v["items"].as_array().unwrap().iter().find(|i| i["id"] == "check-update").unwrap();
        assert!(check_update.get("disabled").is_none()); // 缺省不出现
        let s = MenuState {
            upgrade_running: true,
            ..default_state()
        };
        let v = serde_json::to_value(build_snapshot(&zh(), &s)).unwrap();
        let check_update = v["items"].as_array().unwrap().iter().find(|i| i["id"] == "check-update").unwrap();
        assert_eq!(check_update["disabled"], true);
    }
}
