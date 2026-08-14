//! 导航拦截:webview 内导航的放行判定与外部链接转交系统浏览器(issue #15)。
//!
//! 两条拦截路径(tauri 2.11.5 / wry 0.55.1 源码确认):
//! - 顶层导航(普通链接点击 / location 跳转 / 壳的 Rust 侧 navigate):
//!   `on_navigation` 回调,返回 false 取消导航。该回调只存在于
//!   `WebviewWindowBuilder`,故主窗口改为 setup 中用 `WebviewWindowBuilder::
//!   from_config` 创建(config 保留 `create: false`,窗口几何等单一事实来源
//!   仍在 tauri.conf.json)。
//! - 新窗口请求(`window.open` / `target=_blank`):`on_new_window` 回调。wry
//!   默认拒绝新窗口(SetHandled(true),链接点了没反应),本模块改为交系统
//!   浏览器后拒绝,补上「无反应」缺掉的体验。
//!
//! 放行规则(纯函数 `should_allow_navigation`,单测守住):
//! - 壳本地页(dev: devUrl / prod: `http://tauri.localhost`)——壳自身 navigate
//!   逻辑(navigate_to_shell 等)的导航目标,必须放行
//! - 当前 dsh 服务自身地址(scheme/host/port 与 DshManager 记录的 dsh URL
//!   一致,host 归一:localhost 等价 127.0.0.1;端口运行时动态解析,#3/#17)
//!
//! 其余 URL(外部链接)不放行,经 tauri-plugin-opener 交系统浏览器。
//!
//! 与 #11 的衔接:CSP/ACL 收的是「渲染与 IPC 面」,本模块收「导航面」;
//! 不重复、不冲突(外部页面进不来 webview,自然碰不到 IPC/事件面)。
//!
//! 导航执行也收敛在本模块(单一事实来源):`navigate_main_window`(显示 + 聚焦 +
//! 取消最小化后导航,原 dsh.rs)——boot 就绪导航 / 升级链就绪导航 / 「稍后/返回」
//! 导航共用;`navigate_to_shell`(导航回外壳本地页,原 update.rs)——托盘动态
//! 菜单项与升级对话框共用。

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Url, WebviewWindowBuilder};

use crate::dsh;

/// 壳本地页 URL(dev: devUrl;prod: `http://tauri.localhost`)。
/// 用 `cfg!(dev)`(tauri CLI 的 DEP_TAURI_DEV,tauri-build 源码确认)而非
/// `debug_assertions`——devUrl 只在 `tauri dev` 下被 tauri 实际使用,
/// `cargo build` debug 直接运行时加载的是 tauri.localhost(生产路径)。
/// 单一事实来源:`navigate_to_shell` 与导航拦截判定共用。
pub fn shell_page_url(app: &AppHandle) -> String {
    if cfg!(dev) {
        app.config()
            .build
            .dev_url
            .clone()
            .map(|u| u.to_string())
            .unwrap_or_else(|| "http://localhost:1420".into())
    } else {
        "http://tauri.localhost".into()
    }
}

/// 导航窗口到指定 URL(显示 + 聚焦 + 取消最小化),返回是否成功。
/// 单一事实来源:boot 就绪导航 / 升级链就绪导航 / 「稍后/返回」导航共用。
pub(crate) fn navigate_main_window(app: &AppHandle, url: &str) -> bool {
    let Some(win) = app.get_webview_window("main") else {
        return false;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    match Url::parse(url) {
        Ok(u) => match win.navigate(u) {
            Ok(()) => true,
            Err(e) => {
                log::error!("[navigation] 导航失败 {url}: {e}");
                false
            }
        },
        Err(e) => {
            log::error!("[navigation] URL 解析失败 {url}: {e}");
            false
        }
    }
}

/// 导航窗口回外壳本地页(dev 为 devUrl,prod 为 `http://tauri.localhost`;
/// #3 §5 的导航函数)。托盘动态菜单项与手动检查对话框「升级」共用。
/// URL 单一事实来源在 shell_page_url(#15 导航拦截判定同源共用)。
pub(crate) fn navigate_to_shell(app: &AppHandle) {
    let url = shell_page_url(app);
    log::info!("[navigation] 导航回外壳本地页: {url}");
    navigate_main_window(app, &url);
}

/// 纯函数:候选导航 URL 是否允许在 webview 内加载(返回 false = 调用方应交
/// 系统浏览器)。生产路径:主窗口 `on_navigation` 回调。
pub fn should_allow_navigation(candidate: &Url, shell_page: &str, dsh_service: Option<&str>) -> bool {
    same_origin_as(candidate, shell_page)
        || dsh_service.is_some_and(|d| same_origin_as(candidate, d))
}

/// 同 origin 判定:scheme + 规范化 host(localhost→127.0.0.1)+ 端口。
/// 非 http(s) 协议(mailto:/file:/data: 等)一律视为外部——webview 内没有
/// 处理程序,交系统浏览器是正确去向。
fn same_origin_as(url: &Url, base: &str) -> bool {
    let Ok(base) = Url::parse(base) else {
        return false;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    url.scheme() == base.scheme()
        && canonical_host(url.host_str()) == canonical_host(base.host_str())
        && url.port() == base.port()
}

/// 回环 host 归一:dsh 页面内链接可能写 localhost 而非 127.0.0.1,两者指向同一服务。
fn canonical_host(host: Option<&str>) -> Option<&str> {
    host.map(|h| if h == "localhost" { "127.0.0.1" } else { h })
}

/// 创建主窗口并挂导航拦截。
/// config 中 main 窗口 `create: false`(见 tauri.conf.json):on_navigation /
/// on_new_window 只存在于 builder,故用 `from_config` 从 config 重建(boot
/// 启动页 / 窗口几何等仍由 config 定义,单一事实来源不变)。
pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let conf = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .expect("tauri.conf.json 必须定义 main 窗口(#15 导航拦截需要 builder 挂回调)");
    let nav_app = app.clone();
    let win = WebviewWindowBuilder::from_config(app, conf)?
        .on_navigation(move |url| {
            // try_state:回调可能在 DshManager manage 之前触发(初始加载),
            // 无记录时退化为「只放行壳本地页」,不会误放外部地址
            let manager = nav_app.try_state::<dsh::DshManager>();
            let allowed = should_allow_navigation(
                url,
                &shell_page_url(&nav_app),
                manager.as_deref().and_then(dsh::dsh_url).as_deref(),
            );
            log::info!("[navigation] 导航判定 url={url} allowed={allowed}");
            if !allowed {
                log::info!("[navigation] 拦截外部链接 → 系统浏览器: {url}");
                open_external(url.as_str());
            }
            allowed
        })
        .on_new_window(move |url, _features| {
            // 新窗口请求(window.open / target=_blank):壳只维护一个主窗口,
            // 一律交系统浏览器后拒绝(wry 默认也拒绝,这里补上浏览器打开)
            log::info!("[navigation] 拦截新窗口请求 → 系统浏览器: {url}");
            open_external(url.as_str());
            NewWindowResponse::Deny
        })
        .build()?;
    let _ = win;
    Ok(())
}

/// 经 tauri-plugin-opener 打开外部 URL(系统默认浏览器),失败仅记日志。
fn open_external(url: &str) {
    if let Err(e) = tauri_plugin_opener::open_url(url, None::<&str>) {
        log::error!("[navigation] 打开外部链接失败 {url}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL_PROD: &str = "http://tauri.localhost";
    const DSH: &str = "http://127.0.0.1:53839";

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn allows_dsh_service_own_address() {
        // 生产路径:boot 就绪导航(含路径/查询)——dsh 服务自身地址放行
        assert!(should_allow_navigation(&url(DSH), SHELL_PROD, Some(DSH)));
        assert!(should_allow_navigation(&url("http://127.0.0.1:53839/"), SHELL_PROD, Some(DSH)));
        assert!(should_allow_navigation(
            &url("http://127.0.0.1:53839/conversation/42?x=1"),
            SHELL_PROD,
            Some(DSH)
        ));
    }

    #[test]
    fn treats_localhost_as_dsh_service() {
        // dsh 页面内可能写 localhost,与 127.0.0.1 同一服务,同样放行
        assert!(should_allow_navigation(&url("http://localhost:53839/"), SHELL_PROD, Some(DSH)));
    }

    #[test]
    fn allows_shell_page() {
        // 壳自身导航(navigate_to_shell / dev devUrl)不受拦截
        assert!(should_allow_navigation(&url(SHELL_PROD), SHELL_PROD, None));
        assert!(should_allow_navigation(
            &url("http://tauri.localhost/"),
            SHELL_PROD,
            Some(DSH)
        ));
        assert!(should_allow_navigation(&url("http://localhost:1420/"), "http://localhost:1420", None));
    }

    #[test]
    fn rejects_external_links() {
        // 外部 https 链接 → 系统浏览器
        assert!(!should_allow_navigation(
            &url("https://github.com/Buktal/deepseek-desktop"),
            SHELL_PROD,
            Some(DSH)
        ));
        // 其它本地端口不是 dsh 服务
        assert!(!should_allow_navigation(&url("http://127.0.0.1:9000/"), SHELL_PROD, Some(DSH)));
        // 端口不同不算同源
        assert!(!should_allow_navigation(&url("http://127.0.0.1:53840/"), SHELL_PROD, Some(DSH)));
        // dsh 未就绪(无记录)时,非壳页一律拦截
        assert!(!should_allow_navigation(&url("http://127.0.0.1:53839/"), SHELL_PROD, None));
        // 非 http(s) 协议(mailto: 等)无 webview 处理程序 → 系统浏览器
        assert!(!should_allow_navigation(&url("mailto:test@example.com"), SHELL_PROD, Some(DSH)));
    }
}
