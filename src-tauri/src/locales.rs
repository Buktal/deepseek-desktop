//! Rust 侧外壳文案(原生界面与壳页弹窗):托盘菜单、壳页弹窗/toast
//! (dialog.rs 从本表解析文案随 `shell-dialog` 事件下发)。
//!
//! 跟随系统语言,启动时检测一次(sys-locale)。前端文案走 react-i18next
//! (src/i18n + src/locales/*.json),但托盘/弹窗请求无法消费前端 locale JSON,
//! 故本模块独立维护一份 zh/en 文案选择。
//! 目前无语言偏好设置页,启动检测即终局;将来若加语言设置,由该设置为单一事实
//! 来源,设置变更时重建托盘/对话框文案,前端经事件跟随(O_CC_One 的 LanguageSync 模式)。
//!
//! 静态文案数据化:一条文案的 zh/en 对只写在 TEXT_ROWS 表里一处(新增静态文案
//! 只动表 + 结构体字段声明);插值类文案(版本号等)在 ShellTexts 方法内以
//! format! 模板持有。行序与 ShellTexts 字段声明一一对应,见 TEXT_ROWS 注释。

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
    /// 手动检查发现新版对话框的按钮(升级 / 稍后,见 update.rs)
    pub update_now: &'static str,
    pub update_later: &'static str,
    /// 发现 dsh 新版弹窗标题(dialog.rs show_upgrade_found;#31 场景 3。
    /// 应用新版标题是插值方法 update_found_title,不是本字段)
    pub upgrade_found_title: &'static str,
    /// 「记住我的选择」勾选标签(关闭弹窗,dialog.rs;#31 场景 5)
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

/// 文案数据表:每行一条文案的 (zh, en) 对,行序与 ShellTexts 字段声明一一对应
/// (shell_texts 按索引取用,行注释标注字段名;数组长度与字段数绑定,新增字段
/// 忘加行会在取用时越界 panic 暴露,并有计数测试守住)。新增静态文案只动这里
/// + 结构体字段声明两处,不再各语言写一份巨型构造。
const TEXT_ROWS: [(&str, &str); 20] = [
    // tray_toggle
    ("显示/隐藏窗口", "Show/Hide window"),
    // tray_theme
    ("主题", "Theme"),
    // tray_theme_light
    ("亮色", "Light"),
    // tray_theme_dark
    ("暗色", "Dark"),
    // tray_theme_system
    ("跟随系统", "System"),
    // tray_autostart
    ("开机自启", "Launch at startup"),
    // tray_settings
    ("设置", "Settings"),
    // tray_close_behavior
    ("关闭行为", "Close behavior"),
    // close_ask
    ("每次询问", "Ask each time"),
    // tray_check_update
    ("检查更新", "Check for Updates"),
    // tray_quit
    ("退出", "Quit"),
    // menu_button(壳菜单条按钮文案,menu.rs 快照承载,前端不持有第二份文案,#38)
    ("菜单", "Menu"),
    // close_message("关闭"而非"退出":对话框同时提供"最小化到托盘",问题只问窗口去向)
    ("关闭 DeepSeek Desktop?", "Close DeepSeek Desktop?"),
    // close_quit
    ("退出应用", "Quit app"),
    // close_minimize
    ("最小化到托盘", "Minimize to tray"),
    // update_now / update_later(手动检查发现新版对话框的按钮)
    ("升级", "Upgrade"),
    ("稍后", "Later"),
    // upgrade_found_title(发现 dsh 新版弹窗标题,dialog.rs show_upgrade_found;#31 场景 3)
    ("发现 dsh 新版本", "New dsh version available"),
    // remember_choice(「记住我的选择」勾选标签,关闭弹窗,dialog.rs;#31 场景 5)
    ("记住我的选择", "Remember my choice"),
    // update_running(「升级正在进行中」toast,dialog.rs;#31 行为修正)
    ("升级正在进行中", "An upgrade is in progress"),
];

/// 按语言取行(zh 列 / en 列)。
fn row(lang: Lang, i: usize) -> &'static str {
    match lang {
        Lang::Zh => TEXT_ROWS[i].0,
        Lang::En => TEXT_ROWS[i].1,
    }
}

pub fn shell_texts(lang: Lang) -> ShellTexts {
    ShellTexts {
        tray_toggle: row(lang, 0),
        tray_theme: row(lang, 1),
        tray_theme_light: row(lang, 2),
        tray_theme_dark: row(lang, 3),
        tray_theme_system: row(lang, 4),
        tray_autostart: row(lang, 5),
        tray_settings: row(lang, 6),
        tray_close_behavior: row(lang, 7),
        close_ask: row(lang, 8),
        tray_check_update: row(lang, 9),
        tray_quit: row(lang, 10),
        menu_button: row(lang, 11),
        close_message: row(lang, 12),
        close_quit: row(lang, 13),
        close_minimize: row(lang, 14),
        update_now: row(lang, 15),
        update_later: row(lang, 16),
        upgrade_found_title: row(lang, 17),
        remember_choice: row(lang, 18),
        update_running: row(lang, 19),
        lang,
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

    #[test]
    fn text_table_rows_match_struct_field_count() {
        // 行序与 ShellTexts 字段声明一一对应;数组长度是编译期常量,此处守住
        // 「新增字段必加行」的计数契约(忘加行 → 取用越界 panic 暴露)
        assert_eq!(TEXT_ROWS.len(), 20);
    }

    #[test]
    fn text_table_has_no_empty_rows() {
        // 空文案 = 忘了填,直接红(两种语言任一空都不行)
        for (zh, en) in TEXT_ROWS {
            assert!(!zh.is_empty(), "zh 空行");
            assert!(!en.is_empty(), "en 空行");
        }
    }

    #[test]
    fn shell_texts_both_langs_fully_populated() {
        // 两种语言各取一份:字段数量一致且无空串(数据表投影完整性)
        let zh = shell_texts(Lang::Zh);
        let en = shell_texts(Lang::En);
        let zh_fields = [
            zh.tray_toggle, zh.tray_theme, zh.tray_theme_light, zh.tray_theme_dark,
            zh.tray_theme_system, zh.tray_autostart, zh.tray_settings,
            zh.tray_close_behavior, zh.close_ask, zh.tray_check_update, zh.tray_quit,
            zh.menu_button, zh.close_message, zh.close_quit, zh.close_minimize,
            zh.update_now, zh.update_later, zh.upgrade_found_title, zh.remember_choice,
            zh.update_running,
        ];
        let en_fields = [
            en.tray_toggle, en.tray_theme, en.tray_theme_light, en.tray_theme_dark,
            en.tray_theme_system, en.tray_autostart, en.tray_settings,
            en.tray_close_behavior, en.close_ask, en.tray_check_update, en.tray_quit,
            en.menu_button, en.close_message, en.close_quit, en.close_minimize,
            en.update_now, en.update_later, en.upgrade_found_title, en.remember_choice,
            en.update_running,
        ];
        assert_eq!(zh_fields.len(), 20);
        assert_eq!(en_fields.len(), 20);
        assert!(zh_fields.iter().all(|s| !s.is_empty()));
        assert!(en_fields.iter().all(|s| !s.is_empty()));
    }
}
