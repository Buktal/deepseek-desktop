//! 壳页弹窗请求与回答闭环(#31 拍板 / #39 施工):Rust 侧阻塞式原生 dialog 的
//! Web UI 化。
//!
//! 机制(与菜单快照同构,#31):Rust `emit "shell-dialog"` → 壳页 ShellDialogs
//! 组件渲染(AlertDialog / toast)→ 用户选择 `invoke("shell_dialog_respond",
//! {kind, choice, remember})` → 本模块分发到统一动作表(dispatch_dialog_response,
//! 与托盘 on_menu_event / menu_action 同一张动作表)——「关闭三选」的防双触发
//! 守卫(DIALOG_SHOWN)与响应分发闭环同文件。
//!
//! 文案由本模块从 locales::ShellTexts 解析后放进请求载荷——前端不持有
//! 第二份文案表(与菜单快照同原则);按钮 id 由 DialogChoice 单一事实源
//! (as_str 构造按钮、反序列化回流分发,两侧不会漂移),次序即视觉次序,
//! 强调(疑点 3 结论)随按钮下发。
//!
//! 弹窗请求按 kind 分两类:
//! - dialog 类(update-found / upgrade-found / close-ask):壳页 AlertDialog,
//!   用户选择后必须 respond;
//! - toast 类(toast-up-to-date / toast-check-failed / toast-upgrade-running):
//!   信息性无决策,壳页 Sonner 展示,无需 respond。
//!
//! kind/choice 是 enum 直通:shell_dialog_respond 参数经 serde 反序列化,
//! 未知 wire 值在边界报错(漂移从静默 no-op 变可见错误),不再回落 String
//! 靠 `_ => {}` 吞掉。
//!
//! 触发方窗口不可见时(托盘触发检查):emit 前统一 show 窗口(原 navigate_to_shell
//! 的 show 语义保留,#31 拍板)。

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::close::CloseBehavior;
use crate::{close, dsh, locales, tray, update, upgrade};

/// 弹窗请求类型(序列化 kebab-case 串,前端按 kind 分派渲染;
/// 反序列化拒绝未知值)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellDialogKind {
    /// 发现应用新版(AlertDialog [升级][稍后],notes 承载 release notes 原文)
    UpdateFound,
    /// 发现 dsh 新版(AlertDialog [升级][稍后])
    UpgradeFound,
    /// 关闭二选(AlertDialog [最小化到托盘(默认)][退出] + 记住勾选;无取消
    /// 按钮——Esc/遮罩点击即取消,经前端 respond("cancel") 回流)
    CloseAsk,
    /// 已是最新(toast,合并报告应用 + dsh 版本)
    ToastUpToDate,
    /// 检查失败(toast)
    ToastCheckFailed,
    /// 升级流水线在途,手动检查被拒(toast,#31 行为修正:消除静默 no-op)
    ToastUpgradeRunning,
}

/// 弹窗按钮选择(用户点击按钮 / Esc 遮罩取消 → respond 回传;反序列化拒绝
/// 未知值,wire 形态与按钮 id 一一对应,见 as_str)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DialogChoice {
    /// 「升级」(update-found / upgrade-found)
    Upgrade,
    /// 「稍后」(update-found / upgrade-found)
    Later,
    /// 「最小化到托盘」(close-ask)
    Minimize,
    /// 「退出」(close-ask)
    Quit,
    /// 取消(close-ask 无取消按钮;前端 Esc/遮罩按此语义回流)
    Cancel,
}

impl DialogChoice {
    /// 按钮 id 的单一事实来源:构造按钮(as_str)与 respond 反序列化(lowercase)
    /// 共用同一映射,两侧不会漂移。
    pub fn as_str(self) -> &'static str {
        match self {
            DialogChoice::Upgrade => "upgrade",
            DialogChoice::Later => "later",
            DialogChoice::Minimize => "minimize",
            DialogChoice::Quit => "quit",
            DialogChoice::Cancel => "cancel",
        }
    }
}

/// 按钮视觉强调(疑点 3 结论随按钮下发:默认动作 primary,次级 outline,
/// 取消 ghost;「退出」不用 destructive——关闭确认无数据破坏语义)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DialogButtonVariant {
    Primary,
    Outline,
    Ghost,
}

/// 弹窗按钮:id = Rust 动作表的事实源(respond 回传),label = 已解析文案。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogButton {
    pub id: String,
    pub label: String,
    pub variant: DialogButtonVariant,
}

/// `shell-dialog` 事件载荷(前端 ShellDialogs 渲染的全部信息)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellDialogRequest {
    pub kind: ShellDialogKind,
    /// 标题(dialog 类;toast 类缺省)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 正文(dialog 类 = 正文;toast 类 = toast 内容)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 决策按钮(次序即视觉次序;toast 类为空)
    pub buttons: Vec<DialogButton>,
    /// 发现应用新版弹窗的 release notes 原文(前端 summarizeReleaseNotes 复用)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// 关闭弹窗的「记住我的选择」勾选标签(仅 close-ask)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remember_label: Option<String>,
}

/// 关闭对话框防双触发(CloseRequested 在 webview 与 window 层各触发一次)。
/// 与分发同文件:「关闭三选」闭环单文件(弹窗回答时在 dispatch_dialog_response
/// 复位)。
static DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);

pub(crate) fn try_show_dialog() -> bool {
    DIALOG_SHOWN
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

fn reset_dialog_flag() {
    DIALOG_SHOWN.store(false, Ordering::SeqCst);
}

/// 弹窗按钮:label = 已解析文案,id = DialogChoice(respond 回传分发的事实源)。
fn button(choice: DialogChoice, label: String, variant: DialogButtonVariant) -> DialogButton {
    DialogButton {
        id: choice.as_str().into(),
        label,
        variant,
    }
}

/// emit 前的公共前置:show 窗口(托盘触发检查等场景窗口可能隐藏;
/// 窗口已显示时幂等),再推 `shell-dialog` 事件。
fn emit(app: &AppHandle, request: ShellDialogRequest) {
    tray::show_main_window(app);
    log::info!("[dialog] 弹窗请求 → {:?}", request.kind);
    let _ = app.emit_to("main", "shell-dialog", &request);
}

/// AlertDialog 请求构造(dialog 类六个字段的最小差异收敛:kind / 标题 / 正文 /
/// 按钮 / notes / remember_label;调用点只传差异)。
fn alert(
    app: &AppHandle,
    kind: ShellDialogKind,
    title: String,
    message: Option<String>,
    buttons: Vec<DialogButton>,
    notes: Option<String>,
    remember_label: Option<String>,
) {
    emit(
        app,
        ShellDialogRequest {
            kind,
            title: Some(title),
            message,
            buttons,
            notes,
            remember_label,
        },
    );
}

/// toast 请求构造(toast 类最小差异:kind + 内容;无按钮/标题/可选字段)。
fn toast(app: &AppHandle, kind: ShellDialogKind, message: String) {
    emit(
        app,
        ShellDialogRequest {
            kind,
            title: None,
            message: Some(message),
            buttons: vec![],
            notes: None,
            remember_label: None,
        },
    );
}

/// 手动检查发现应用新版:AlertDialog 标题「发现新版 vX」,正文 = 中断影响明示,
/// notes = release notes 原文摘要(前端复用 summarizeReleaseNotes)。#31 场景 1。
pub fn show_update_found(app: &AppHandle, version: &str, current: &str, notes: Option<&str>) {
    let t = locales::shell_texts(locales::detect_lang());
    alert(
        app,
        ShellDialogKind::UpdateFound,
        t.update_found_title(version),
        Some(t.update_found_message(version, current)),
        vec![
            button(DialogChoice::Upgrade, t.update_now.into(), DialogButtonVariant::Primary),
            button(DialogChoice::Later, t.update_later.into(), DialogButtonVariant::Ghost),
        ],
        notes.map(String::from),
        None,
    );
}

/// 手动检查发现 dsh 新版:AlertDialog [升级][稍后],确认后进 dsh 升级
/// 流水线(全屏覆盖层归 #32)。#31 场景 3。
pub fn show_upgrade_found(app: &AppHandle, version: &str, current: &str) {
    let t = locales::shell_texts(locales::detect_lang());
    alert(
        app,
        ShellDialogKind::UpgradeFound,
        t.upgrade_found_title.into(),
        Some(t.upgrade_found_message(version, current)),
        vec![
            button(DialogChoice::Upgrade, t.update_now.into(), DialogButtonVariant::Primary),
            button(DialogChoice::Later, t.update_later.into(), DialogButtonVariant::Ghost),
        ],
        None,
        None,
    );
}

/// 关闭二选(首次 / 设置为每次询问):AlertDialog [最小化到托盘(默认)][退出]
/// + 「记住我的选择」勾选框;#31 场景 5。lib.rs close handler 触发。
///
/// 取消按钮已退役:遮罩点击/Esc 由前端按 cancel 语义 respond 回流复位守卫。
pub fn show_close_ask(app: &AppHandle) {
    let t = locales::shell_texts(locales::detect_lang());
    alert(
        app,
        ShellDialogKind::CloseAsk,
        t.close_message.into(),
        None,
        vec![
            button(
                DialogChoice::Minimize,
                t.close_minimize.into(),
                DialogButtonVariant::Primary,
            ),
            button(DialogChoice::Quit, t.close_quit.into(), DialogButtonVariant::Outline),
        ],
        None,
        Some(t.remember_choice.into()),
    );
}

/// 手动检查无新版:toast 合并报告应用 + dsh 版本(信息性无决策,不弹窗打断)。
/// #31 场景 2。
pub fn toast_up_to_date(app: &AppHandle, dsh_version: Option<&str>) {
    let current = app.package_info().version.to_string();
    let t = locales::shell_texts(locales::detect_lang());
    toast(
        app,
        ShellDialogKind::ToastUpToDate,
        t.update_up_to_date_message(&current, dsh_version),
    );
}

/// 手动检查失败:toast。#31 场景 4。
pub fn toast_check_failed(app: &AppHandle) {
    let t = locales::shell_texts(locales::detect_lang());
    toast(
        app,
        ShellDialogKind::ToastCheckFailed,
        t.check_update_failed_message().into(),
    );
}

/// 升级流水线在途,手动「检查更新」被拒:toast 可见反馈
/// (#31 行为修正:原静默 no-op 消除)。
pub fn toast_upgrade_running(app: &AppHandle) {
    let t = locales::shell_texts(locales::detect_lang());
    toast(app, ShellDialogKind::ToastUpgradeRunning, t.update_running.into());
}

/// 弹窗回答分发(#31 拍板:与托盘 on_menu_event / menu_action 同一张动作表)。
/// kind/choice 是 enum 直通:未知 wire 值在命令反序列化边界报错(漂移从静默
/// no-op 变可见错误),此处只处理已知组合;已知但无操作的组合(「稍后」等)
/// 显式 no-op(保持升级槽位/状态现状,等待下一次触发)。
pub(crate) fn dispatch_dialog_response(
    app: &AppHandle,
    kind: ShellDialogKind,
    choice: DialogChoice,
    remember: bool,
) {
    match (kind, choice) {
        // 发现应用新版 [升级] → 显示窗口 + 自动开始下载(#3:确认即授权,
        // 不二次确认;下载进度浮层随 update-state Downloading 自动出现)
        (ShellDialogKind::UpdateFound, DialogChoice::Upgrade) => {
            log::info!("[dialog] 弹窗[升级](应用) → 显示窗口 + 自动开始下载");
            if let (Some(u), Some(d)) = (
                app.try_state::<update::UpdateManager>(),
                app.try_state::<dsh::DshManager>(),
            ) {
                u.inner().apply_now(d.inner());
            }
        }
        // 发现 dsh 新版 [升级] → 显示窗口 + 自动开始流水线(#3 §1:确认即授权,
        // 不二次确认;流水线进入 Active 后升级覆盖层自动出现,#32)
        (ShellDialogKind::UpgradeFound, DialogChoice::Upgrade) => {
            log::info!("[dialog] 弹窗[升级](dsh) → 显示窗口 + 自动开始流水线");
            if let (Some(u), Some(d)) = (
                app.try_state::<upgrade::UpgradeManager>(),
                app.try_state::<dsh::DshManager>(),
            ) {
                u.inner().confirm_start(d.inner());
            }
        }
        // 关闭二选(首次 / 每次询问,#31 场景 5):执行去向;勾选「记住我的选择」
        // → close::set 持久化(下次直接执行不再弹,close.rs 单一事实源)
        (ShellDialogKind::CloseAsk, DialogChoice::Minimize) => {
            reset_dialog_flag();
            close::execute(app, CloseBehavior::Minimize);
            if remember {
                log::info!("[dialog] 关闭选择[最小化到托盘] + 记住 → 持久化");
                close::set(app, CloseBehavior::Minimize);
            }
        }
        (ShellDialogKind::CloseAsk, DialogChoice::Quit) => {
            reset_dialog_flag();
            close::execute(app, CloseBehavior::Quit);
            if remember {
                log::info!("[dialog] 关闭选择[退出] + 记住 → 持久化");
                close::set(app, CloseBehavior::Quit);
            }
        }
        (ShellDialogKind::CloseAsk, DialogChoice::Cancel) => {
            // 取消(前端 Esc/遮罩):保持现状(窗口未关闭),仅复位防双触发守卫
            reset_dialog_flag();
        }
        // 「稍后」与其余组合无操作(保持状态现状,等待下一次触发)
        _ => {}
    }
}

/// 弹窗回答命令:前端用户选择 → 本模块统一分发(与托盘 on_menu_event /
/// menu_action 同一张动作表,#31)。kind/choice 未知 wire 值在反序列化边界
/// 报错,不静默吞掉。
#[tauri::command]
pub async fn shell_dialog_respond(
    app: tauri::AppHandle,
    kind: ShellDialogKind,
    choice: DialogChoice,
    remember: Option<bool>,
) -> Result<(), String> {
    dispatch_dialog_response(&app, kind, choice, remember.unwrap_or(false));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locales::ShellTexts;

    fn zh() -> ShellTexts {
        locales::shell_texts(locales::Lang::Zh)
    }

    #[test]
    fn update_found_request_carries_notes_and_buttons() {
        // 线上契约(shell-dialog 事件):kind 小写串,字段 camelCase,
        // 可选字段缺省不出现;按钮 id 与变体随请求下发
        let req = ShellDialogRequest {
            kind: ShellDialogKind::UpdateFound,
            title: Some(zh().update_found_title("0.5.0")),
            message: Some(zh().update_found_message("0.5.0", "0.4.0")),
            buttons: vec![
                button(DialogChoice::Upgrade, zh().update_now.into(), DialogButtonVariant::Primary),
                button(DialogChoice::Later, zh().update_later.into(), DialogButtonVariant::Ghost),
            ],
            notes: Some("- fix a\n- fix b".into()),
            remember_label: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["kind"], "update-found");
        assert_eq!(v["title"], "发现新版 v0.5.0");
        assert_eq!(v["notes"], "- fix a\n- fix b");
        assert_eq!(v["buttons"][0]["id"], "upgrade");
        assert_eq!(v["buttons"][0]["variant"], "primary");
        assert_eq!(v["buttons"][1]["id"], "later");
        assert!(v.get("rememberLabel").is_none());
    }

    #[test]
    fn close_ask_request_carries_remember_label() {
        let req = ShellDialogRequest {
            kind: ShellDialogKind::CloseAsk,
            title: Some(zh().close_message.into()),
            message: None,
            buttons: vec![
                button(DialogChoice::Minimize, zh().close_minimize.into(), DialogButtonVariant::Primary),
                button(DialogChoice::Quit, zh().close_quit.into(), DialogButtonVariant::Outline),
            ],
            notes: None,
            remember_label: Some(zh().remember_choice.into()),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["kind"], "close-ask");
        assert_eq!(v["rememberLabel"], "记住我的选择");
        assert!(v.get("message").is_none()); // 缺省不出现
        // 按钮次序即视觉次序(#31 拍板):最小化到托盘(primary) → 退出。
        // 无取消按钮:遮罩点击/Esc 经前端按 cancel 语义 respond 回流
        let ids: Vec<&str> = req
            .buttons
            .iter()
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(ids, ["minimize", "quit"]);
        assert_eq!(req.buttons[0].variant, DialogButtonVariant::Primary);
        assert_eq!(req.buttons[1].variant, DialogButtonVariant::Outline);
    }

    #[test]
    fn dialog_choice_wire_roundtrip() {
        // 不变量:按钮 id(as_str,构造侧)↔ respond 反序列化(lowercase)一一对应。
        // 前端契约:close-ask 无「取消」按钮,遮罩/Esc 仍按 "cancel" 语义回流
        for choice in [
            DialogChoice::Upgrade,
            DialogChoice::Later,
            DialogChoice::Minimize,
            DialogChoice::Quit,
            DialogChoice::Cancel,
        ] {
            let wire = serde_json::to_string(&choice).unwrap();
            assert_eq!(wire, format!("\"{}\"", choice.as_str()), "序列化形态必须与按钮 id 一致");
            assert_eq!(serde_json::from_str::<DialogChoice>(&wire).unwrap(), choice);
        }
    }

    #[test]
    fn dialog_choice_rejects_unknown_wire_values() {
        // 漂移从静默 no-op 变边界报错:未知 choice/kind 串反序列化失败
        assert!(serde_json::from_str::<DialogChoice>("\"upgrade-now\"").is_err());
        assert!(serde_json::from_str::<DialogChoice>("\"\"").is_err());
        assert!(serde_json::from_str::<ShellDialogKind>("\"update-found-v2\"").is_err());
        assert!(serde_json::from_str::<ShellDialogKind>("\"\"").is_err());
    }

    #[test]
    fn toast_requests_have_no_buttons_or_title() {
        let req = ShellDialogRequest {
            kind: ShellDialogKind::ToastUpgradeRunning,
            title: None,
            message: Some(zh().update_running.into()),
            buttons: vec![],
            notes: None,
            remember_label: None,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["kind"], "toast-upgrade-running");
        assert_eq!(v["message"], "升级正在进行中");
        assert!(v.get("title").is_none());
        assert_eq!(v["buttons"].as_array().unwrap().len(), 0);
    }
}
