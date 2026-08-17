//! 壳页弹窗请求(#31 拍板 / #39 施工):Rust 侧阻塞式原生 dialog 的 Web UI 化。
//!
//! 机制(与菜单快照同构,#31):Rust `emit "shell-dialog"` → 壳页 ShellDialogs
//! 组件渲染(AlertDialog / toast)→ 用户选择 `invoke("shell_dialog_respond",
//! {kind, choice, remember})` → 分发到统一动作表(tray.rs dispatch_dialog_response,
//! 与托盘 on_menu_event / menu_action 同一张动作表)。
//!
//! 文案由本模块从 locales::ShellTexts 解析后放进请求载荷——前端不持有
//! 第二份文案表(与菜单快照同原则);按钮 id 是 Rust 动作表的唯一事实源,
//! 次序即视觉次序,强调(疑点 3 结论)随按钮下发。
//!
//! 弹窗请求按 kind 分两类:
//! - dialog 类(update-found / upgrade-found / close-ask):壳页 AlertDialog,
//!   用户选择后必须 respond;
//! - toast 类(toast-up-to-date / toast-check-failed / toast-upgrade-running):
//!   信息性无决策,壳页 Sonner 展示,无需 respond。
//!
//! 触发方窗口不可见时(托盘触发检查):emit 前统一 show 窗口(原 navigate_to_shell
//! 的 show 语义保留,#31 拍板)。

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::locales;
use crate::tray;

/// 弹窗请求类型(序列化 kebab-case 串,前端按 kind 分派渲染)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellDialogKind {
    /// 发现应用新版(AlertDialog [升级][稍后],notes 承载 release notes 原文)
    UpdateFound,
    /// 发现 dsh 新版(AlertDialog [升级][稍后])
    UpgradeFound,
    /// 关闭三选(AlertDialog [最小化到托盘][退出][取消] + 记住勾选)
    CloseAsk,
    /// 已是最新(toast,合并报告应用 + dsh 版本)
    ToastUpToDate,
    /// 检查失败(toast)
    ToastCheckFailed,
    /// 升级流水线在途,手动检查被拒(toast,#31 行为修正:消除静默 no-op)
    ToastUpgradeRunning,
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
    /// 关闭三选的「记住我的选择」勾选标签(仅 close-ask)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remember_label: Option<String>,
}

fn button(id: &str, label: String, variant: DialogButtonVariant) -> DialogButton {
    DialogButton {
        id: id.into(),
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

/// 手动检查发现应用新版:AlertDialog 标题「发现新版 vX」,正文 = 中断影响明示,
/// notes = release notes 原文摘要(前端复用 summarizeReleaseNotes)。#31 场景 1。
pub fn show_update_found(app: &AppHandle, version: &str, current: &str, notes: Option<&str>) {
    let t = locales::shell_texts(locales::detect_lang());
    emit(
        app,
        ShellDialogRequest {
            kind: ShellDialogKind::UpdateFound,
            title: Some(t.update_found_title(version)),
            message: Some(t.update_found_message(version, current)),
            buttons: vec![
                button("upgrade", t.update_now.into(), DialogButtonVariant::Primary),
                button("later", t.update_later.into(), DialogButtonVariant::Ghost),
            ],
            notes: notes.map(String::from),
            remember_label: None,
        },
    );
}

/// 手动检查发现 dsh 新版:AlertDialog [升级][稍后],确认后进 dsh 升级
/// 流水线(全屏覆盖层归 #32)。#31 场景 3。
pub fn show_upgrade_found(app: &AppHandle, version: &str, current: &str) {
    let t = locales::shell_texts(locales::detect_lang());
    emit(
        app,
        ShellDialogRequest {
            kind: ShellDialogKind::UpgradeFound,
            title: Some(t.upgrade_found_title.into()),
            message: Some(t.upgrade_found_message(version, current)),
            buttons: vec![
                button("upgrade", t.update_now.into(), DialogButtonVariant::Primary),
                button("later", t.update_later.into(), DialogButtonVariant::Ghost),
            ],
            notes: None,
            remember_label: None,
        },
    );
}

/// 关闭三选(首次 / 设置为每次询问):AlertDialog [最小化到托盘(默认)][退出]
/// [取消] + 「记住我的选择」勾选框;#31 场景 5。lib.rs close handler 触发。
pub fn show_close_ask(app: &AppHandle) {
    let t = locales::shell_texts(locales::detect_lang());
    emit(
        app,
        ShellDialogRequest {
            kind: ShellDialogKind::CloseAsk,
            title: Some(t.close_message.into()),
            message: None,
            buttons: vec![
                button(
                    "minimize",
                    t.close_minimize.into(),
                    DialogButtonVariant::Primary,
                ),
                button("quit", t.close_quit.into(), DialogButtonVariant::Outline),
                button("cancel", t.close_cancel.into(), DialogButtonVariant::Ghost),
            ],
            notes: None,
            remember_label: Some(t.remember_choice.into()),
        },
    );
}

/// 手动检查无新版:toast 合并报告应用 + dsh 版本(信息性无决策,不弹窗打断)。
/// #31 场景 2。
pub fn toast_up_to_date(app: &AppHandle, dsh_version: Option<&str>) {
    let current = app.package_info().version.to_string();
    let t = locales::shell_texts(locales::detect_lang());
    emit(
        app,
        ShellDialogRequest {
            kind: ShellDialogKind::ToastUpToDate,
            title: None,
            message: Some(t.update_up_to_date_message(&current, dsh_version)),
            buttons: vec![],
            notes: None,
            remember_label: None,
        },
    );
}

/// 手动检查失败:toast。#31 场景 4。
pub fn toast_check_failed(app: &AppHandle) {
    let t = locales::shell_texts(locales::detect_lang());
    emit(
        app,
        ShellDialogRequest {
            kind: ShellDialogKind::ToastCheckFailed,
            title: None,
            message: Some(t.check_update_failed_message().into()),
            buttons: vec![],
            notes: None,
            remember_label: None,
        },
    );
}

/// 升级流水线在途,手动「检查更新」被拒:toast 可见反馈
/// (#31 行为修正:原静默 no-op 消除)。
pub fn toast_upgrade_running(app: &AppHandle) {
    let t = locales::shell_texts(locales::detect_lang());
    emit(
        app,
        ShellDialogRequest {
            kind: ShellDialogKind::ToastUpgradeRunning,
            title: None,
            message: Some(t.update_running.into()),
            buttons: vec![],
            notes: None,
            remember_label: None,
        },
    );
}

/// 弹窗回答命令:前端用户选择 → 分发到统一动作表(tray.rs dispatch_dialog_response,
/// 与托盘 on_menu_event / menu_action 同一张表,#31)。kind/choice 未知时无操作。
#[tauri::command]
pub async fn shell_dialog_respond(
    app: tauri::AppHandle,
    kind: String,
    choice: String,
    remember: Option<bool>,
) -> Result<(), String> {
    tray::dispatch_dialog_response(&app, &kind, &choice, remember.unwrap_or(false));
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
                button("upgrade", zh().update_now.into(), DialogButtonVariant::Primary),
                button("later", zh().update_later.into(), DialogButtonVariant::Ghost),
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
                button("minimize", zh().close_minimize.into(), DialogButtonVariant::Primary),
                button("quit", zh().close_quit.into(), DialogButtonVariant::Outline),
                button("cancel", zh().close_cancel.into(), DialogButtonVariant::Ghost),
            ],
            notes: None,
            remember_label: Some(zh().remember_choice.into()),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["kind"], "close-ask");
        assert_eq!(v["rememberLabel"], "记住我的选择");
        assert!(v.get("message").is_none()); // 缺省不出现
        // 按钮次序即视觉次序(#31 拍板):最小化到托盘(primary) → 退出 → 取消
        let ids: Vec<&str> = req
            .buttons
            .iter()
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(ids, ["minimize", "quit", "cancel"]);
        assert_eq!(req.buttons[0].variant, DialogButtonVariant::Primary);
        assert_eq!(req.buttons[1].variant, DialogButtonVariant::Outline);
        assert_eq!(req.buttons[2].variant, DialogButtonVariant::Ghost);
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
