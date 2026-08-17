//! Rust 侧外壳文案(原生界面与壳页弹窗):托盘菜单、壳页弹窗/toast
//! (dialog.rs 从本表解析文案随 `shell-dialog` 事件下发)。
//!
//! 跟随系统语言,启动时检测一次(sys-locale)。前端文案走 react-i18next
//! (src/i18n + src/locales/*.json),但托盘/弹窗请求无法消费前端 locale JSON,
//! 故本模块独立维护一份 zh/en 文案选择。
//! 目前无语言偏好设置页,启动检测即终局;将来若加语言设置,由该设置为单一事实
//! 来源,设置变更时重建托盘/对话框文案,前端经事件跟随(O_CC_One 的 LanguageSync 模式)。

/// 语言判别。`lang_from_locale` 与前端 src/i18n/languages.ts 的 `resolveLanguage`
/// 同规则(zh* → zh,en* → en,其余 → zh)。两个运行时各持一份实现——
/// 各自是所在运行时的单一实现,跨运行时共享的规则以本注释为锚点,改动需同步。
#[derive(Clone, Copy)]
pub enum Lang {
    Zh,
    En,
}

/// 纯函数:locale 串 → 语言(可测)。`get_locale()` 返回形如 "zh-CN"/"en-US" 的串。
pub fn lang_from_locale(locale: Option<&str>) -> Lang {
    match locale {
        Some(l) if l.to_ascii_lowercase().starts_with("zh") => Lang::Zh,
        Some(l) if l.to_ascii_lowercase().starts_with("en") => Lang::En,
        _ => Lang::Zh, // fallback zh
    }
}

/// 检测系统语言(启动时调用一次)。
pub fn detect_lang() -> Lang {
    lang_from_locale(sys_locale::get_locale().as_deref())
}

/// 原生界面文案表。&'static str 让 setup 阶段构建的托盘/对话框闭包无需持有 String;
/// 动态数值(版本号等)经下方 `*_message` / `*_label` 函数插值,禁止拼串(#3 §6)。
#[derive(Clone, Copy)]
pub struct ShellTexts {
    pub tray_toggle: &'static str,
    pub tray_theme: &'static str,
    pub tray_theme_light: &'static str,
    pub tray_theme_dark: &'static str,
    pub tray_theme_system: &'static str,
    pub tray_autostart: &'static str,
    pub tray_settings: &'static str,
    pub tray_close_behavior: &'static str,
    pub close_ask: &'static str,
    pub tray_check_update: &'static str,
    pub tray_quit: &'static str,
    /// 壳菜单条按钮文案(menu.rs 快照承载,前端不持有第二份文案,#38)
    pub menu_button: &'static str,
    pub close_message: &'static str,
    pub close_quit: &'static str,
    pub close_minimize: &'static str,
    pub close_cancel: &'static str,
    /// 手动检查发现新版对话框的按钮(升级 / 稍后,见 update.rs)
    pub update_now: &'static str,
    pub update_later: &'static str,
    /// 发现应用新版弹窗标题(dialog.rs;#31 场景 1「发现新版 vX」)
    pub upgrade_found_title: &'static str,
    /// 「记住我的选择」勾选标签(关闭三选弹窗,dialog.rs;#31 场景 5)
    pub remember_choice: &'static str,
    /// 「升级正在进行中」toast(升级流水线在途时手动检查被拒的可见反馈,
    /// dialog.rs;#31 行为修正:消除静默 no-op)
    pub update_running: &'static str,
    lang: Lang,
}

impl ShellTexts {
    /// 手动检查发现应用新版:弹窗标题「发现新版 vX」(dialog.rs,#31 场景 1)。
    pub fn update_found_title(&self, version: &str) -> String {
        match self.lang {
            Lang::Zh => format!("发现新版 v{version}"),
            Lang::En => format!("New version v{version} available"),
        }
    }

    /// 手动检查发现新版:对话框正文(含中断影响明示,#3 §4)。
    pub fn update_found_message(&self, version: &str, current: &str) -> String {
        match self.lang {
            Lang::Zh => format!(
                "发现新版本 v{version}(当前 v{current})。升级将重启应用与 dsh 服务,当前页面会话会中断;数据保存在本机,不受影响"
            ),
            Lang::En => format!(
                "New version v{version} available (current v{current}). Upgrading will restart the app and the dsh service, interrupting the current page session. Your data stays on this machine"
            ),
        }
    }

    /// 手动检查无新版:对话框正文(合并报告两层升级——dsh 版本已知时一并说明,#17)。
    pub fn update_up_to_date_message(&self, app_version: &str, dsh_version: Option<&str>) -> String {
        match (self.lang, dsh_version) {
            (Lang::Zh, Some(dsh)) => {
                format!("应用与 dsh 均已是最新版本(v{app_version} / v{dsh})")
            }
            (Lang::En, Some(dsh)) => {
                format!("The app and dsh are both up to date (v{app_version} / v{dsh})")
            }
            (Lang::Zh, None) => format!("DeepSeek Desktop 已是最新版本(v{app_version})"),
            (Lang::En, None) => format!("DeepSeek Desktop is up to date (v{app_version})"),
        }
    }

    /// 手动检查发现 dsh 新版:对话框正文(含中断影响明示,#3 §4)。
    pub fn upgrade_found_message(&self, version: &str, current: &str) -> String {
        match self.lang {
            Lang::Zh => format!(
                "发现 dsh 新版本 v{version}(当前 v{current})。升级将重启 dsh 服务,当前页面会话会中断;数据保存在本机,不受影响"
            ),
            Lang::En => format!(
                "New dsh version v{version} available (current v{current}). Upgrading restarts the dsh service and interrupts this page session. Your data stays on this machine"
            ),
        }
    }

    /// 手动检查失败:对话框正文。
    pub fn check_update_failed_message(&self) -> &'static str {
        match self.lang {
            Lang::Zh => "检查更新失败,请稍后重试",
            Lang::En => "Update check failed, please try again later",
        }
    }

    /// 托盘动态菜单项文案:「升级到 vX」(应用自身升级,发现新版时插入菜单,#3 §1)。
    pub fn tray_upgrade_label(&self, version: &str) -> String {
        match self.lang {
            Lang::Zh => format!("升级到 v{version}"),
            Lang::En => format!("Upgrade to v{version}"),
        }
    }

    /// 托盘动态菜单项文案:「升级 dsh 到 vX」(dsh 升级,发现新版时插入菜单,#3 §1)。
    pub fn tray_upgrade_dsh_label(&self, version: &str) -> String {
        match self.lang {
            Lang::Zh => format!("升级 dsh 到 v{version}"),
            Lang::En => format!("Upgrade dsh to v{version}"),
        }
    }

    /// 托盘 tooltip:发现新版时的提示。
    pub fn tray_tooltip_available(&self, version: &str) -> String {
        match self.lang {
            Lang::Zh => format!("发现新版本 v{version}"),
            Lang::En => format!("New version v{version} available"),
        }
    }

    /// 托盘 tooltip:发现 dsh 新版时的提示。
    pub fn tray_tooltip_dsh_available(&self, version: &str) -> String {
        match self.lang {
            Lang::Zh => format!("发现 dsh 新版本 v{version}"),
            Lang::En => format!("New dsh version v{version} available"),
        }
    }
}

pub fn shell_texts(lang: Lang) -> ShellTexts {
    match lang {
        Lang::Zh => ShellTexts {
            tray_toggle: "显示/隐藏窗口",
            tray_theme: "主题",
            tray_theme_light: "亮色",
            tray_theme_dark: "暗色",
            tray_theme_system: "跟随系统",
            tray_autostart: "开机自启",
            tray_settings: "设置",
            tray_close_behavior: "关闭行为",
            close_ask: "每次询问",
            tray_check_update: "检查更新",
            tray_quit: "退出",
            menu_button: "菜单",
            // "关闭"而非"退出":对话框同时提供"最小化到托盘",问题只问窗口去向
            close_message: "关闭 DeepSeek Desktop?",
            close_quit: "退出应用",
            close_minimize: "最小化到托盘",
            close_cancel: "取消",
            update_now: "升级",
            update_later: "稍后",
            upgrade_found_title: "发现 dsh 新版本",
            remember_choice: "记住我的选择",
            update_running: "升级正在进行中",
            lang: Lang::Zh,
        },
        Lang::En => ShellTexts {
            tray_toggle: "Show/Hide window",
            tray_theme: "Theme",
            tray_theme_light: "Light",
            tray_theme_dark: "Dark",
            tray_theme_system: "System",
            tray_autostart: "Launch at startup",
            tray_settings: "Settings",
            tray_close_behavior: "Close behavior",
            close_ask: "Ask each time",
            tray_check_update: "Check for Updates",
            tray_quit: "Quit",
            menu_button: "Menu",
            // "Close" instead of "Quit": the dialog also offers "Minimize to tray",
            // so the question is about the window, not the process
            close_message: "Close DeepSeek Desktop?",
            close_quit: "Quit app",
            close_minimize: "Minimize to tray",
            close_cancel: "Cancel",
            update_now: "Upgrade",
            update_later: "Later",
            upgrade_found_title: "New dsh version available",
            remember_choice: "Remember my choice",
            update_running: "An upgrade is in progress",
            lang: Lang::En,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_from_locale_maps_zh_and_en() {
        assert!(matches!(lang_from_locale(Some("zh-CN")), Lang::Zh));
        assert!(matches!(lang_from_locale(Some("zh-Hant-TW")), Lang::Zh));
        assert!(matches!(lang_from_locale(Some("ZH")), Lang::Zh));
        assert!(matches!(lang_from_locale(Some("en-US")), Lang::En));
        assert!(matches!(lang_from_locale(Some("EN")), Lang::En));
    }

    #[test]
    fn lang_from_locale_falls_back_to_zh() {
        assert!(matches!(lang_from_locale(Some("ja-JP")), Lang::Zh));
        assert!(matches!(lang_from_locale(Some("fr-FR")), Lang::Zh));
        assert!(matches!(lang_from_locale(None), Lang::Zh));
    }
}
