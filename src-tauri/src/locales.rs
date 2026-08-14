//! Rust 侧外壳文案(原生界面):托盘菜单、关闭三选对话框、dsh 意外退出提示。
//!
//! 跟随系统语言,启动时检测一次(sys-locale)。前端文案走 react-i18next
//! (src/i18n + src/locales/*.json),但原生对话框/托盘无法消费前端 locale JSON,
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
    pub tray_check_update: &'static str,
    pub tray_quit: &'static str,
    pub close_message: &'static str,
    pub close_quit: &'static str,
    pub close_minimize: &'static str,
    pub close_cancel: &'static str,
    pub dsh_crashed: &'static str,
    /// 手动检查发现新版对话框的按钮(升级 / 稍后,见 update.rs)
    pub update_now: &'static str,
    pub update_later: &'static str,
    lang: Lang,
}

impl ShellTexts {
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

    /// 手动检查无新版:对话框正文。
    pub fn update_up_to_date_message(&self, current: &str) -> String {
        match self.lang {
            Lang::Zh => format!("DeepSeek Desktop 已是最新版本(v{current})"),
            Lang::En => format!("DeepSeek Desktop is up to date (v{current})"),
        }
    }

    /// 托盘动态菜单项文案:「升级到 vX」(发现新版时插入菜单,#3 §1)。
    pub fn tray_upgrade_label(&self, version: &str) -> String {
        match self.lang {
            Lang::Zh => format!("升级到 v{version}"),
            Lang::En => format!("Upgrade to v{version}"),
        }
    }

    /// 托盘 tooltip:发现新版时的提示。
    pub fn tray_tooltip_available(&self, version: &str) -> String {
        match self.lang {
            Lang::Zh => format!("发现新版本 v{version}"),
            Lang::En => format!("New version v{version} available"),
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
            tray_check_update: "检查更新",
            tray_quit: "退出",
            // "关闭"而非"退出":对话框同时提供"最小化到托盘",问题只问窗口去向
            close_message: "关闭 DeepSeek Desktop?",
            close_quit: "退出应用",
            close_minimize: "最小化到托盘",
            close_cancel: "取消",
            dsh_crashed: "dsh 进程意外退出,请重新启动应用",
            update_now: "升级",
            update_later: "稍后",
            lang: Lang::Zh,
        },
        Lang::En => ShellTexts {
            tray_toggle: "Show/Hide window",
            tray_theme: "Theme",
            tray_theme_light: "Light",
            tray_theme_dark: "Dark",
            tray_theme_system: "System",
            tray_autostart: "Launch at startup",
            tray_check_update: "Check for Updates",
            tray_quit: "Quit",
            // "Close" instead of "Quit": the dialog also offers "Minimize to tray",
            // so the question is about the window, not the process
            close_message: "Close DeepSeek Desktop?",
            close_quit: "Quit app",
            close_minimize: "Minimize to tray",
            close_cancel: "Cancel",
            dsh_crashed: "dsh exited unexpectedly; please restart the app",
            update_now: "Upgrade",
            update_later: "Later",
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
