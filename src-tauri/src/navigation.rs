//! 导航拦截与页面层外链拦截:webview 内导航的放行判定与外部链接转交系统浏览器
//! (issue #15;壳页常驻后的角色调整见下,#29/#36)。
//!
//! 壳页常驻(ADR 0001)后,本模块只剩拦截职责:
//! - 窗口导航函数已退役(navigate_main_window / navigate_to_shell 删除)——壳页
//!   常驻、dsh 以 iframe 嵌入,窗口不再整窗导航。
//! - 顶层导航拦截保留(防壳页自身被整窗导航走):`on_navigation` / `on_new_window`
//!   对 iframe 内导航/新窗口不可依赖(Windows 两者均不触发、三平台分叉,wry#1593),
//!   但对顶层仍是有效防御;Linux 上 iframe 导航会经 on_navigation 上报——放行
//!   规则对 dsh 自身地址放行,故 Linux 上 iframe 内部导航不受影响(见单测)。
//! - 外链拦截移页面层:`initialization_script_for_all_frames` 注入
//!   `EXTERNAL_LINK_SCRIPT`——子帧(dsh iframe)内命中外链 → `window.parent.
//!   postMessage` 回壳页 → 壳页经 tauri-plugin-opener 开系统浏览器(壳页侧
//!   消息解析见 src/lib/externalLinks.ts)。
//!
//! 两条顶层拦截路径(tauri 2.11.5 / wry 0.55.1 源码确认):
//! - 顶层导航(普通链接点击 / location 跳转):`on_navigation` 回调,返回 false
//!   取消导航。该回调只存在于 `WebviewWindowBuilder`,故主窗口改为 setup 中用
//!   `WebviewWindowBuilder::from_config` 创建(config 保留 `create: false`,
//!   窗口几何等单一事实来源仍在 tauri.conf.json)。
//! - 新窗口请求(`window.open` / `target=_blank`):`on_new_window` 回调。wry
//!   默认拒绝新窗口(SetHandled(true),链接点了没反应),本模块改为交系统
//!   浏览器后拒绝,补上「无反应」缺掉的体验。
//!
//! 放行规则(纯函数 `should_allow_navigation`,单测守住):
//! - 壳本地页(dev: devUrl / prod: `http://tauri.localhost`)
//! - 当前 dsh 服务自身地址(scheme/host/port 与 DshManager 记录的 dsh URL
//!   一致,host 归一:localhost 等价 127.0.0.1;端口运行时动态解析,#3/#17)
//!
//! 其余 URL(外部链接)不放行,经 tauri-plugin-opener 交系统浏览器。
//!
//! 与 #11 的衔接:CSP/ACL 收的是「渲染与 IPC 面」,本模块收「导航面」;
//! 不重复、不冲突(外部页面进不来 webview,自然碰不到 IPC/事件面)。

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Url, WebviewWindowBuilder};

use crate::dsh;

/// 壳本地页 URL(dev: devUrl;prod: `http://tauri.localhost`)。
/// 用 `cfg!(dev)`(tauri CLI 的 DEP_TAURI_DEV,tauri-build 源码确认)而非
/// `debug_assertions`——devUrl 只在 `tauri dev` 下被 tauri 实际使用,
/// `cargo build` debug 直接运行时加载的是 tauri.localhost(生产路径)。
/// 单一事实来源:导航拦截判定共用(壳页常驻后不再有「导航回壳页」的调用方)。
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

/// 页面层外链拦截脚本(initialization_script_for_all_frames 注入,所有帧生效)。
///
/// 顶层帧(壳页):`window.parent === window`,直接 no-op——壳页自身链接仍走
/// Rust 侧 on_navigation(壳页顶层导航拦截保留)。
///
/// 子帧(dsh iframe):捕获阶段拦截外链点击(click / auxclick)→ preventDefault,
/// 经 `window.parent.postMessage({type:"open-external",url})` 回壳页,壳页用
/// tauri-plugin-opener 开系统浏览器(协议白名单见前端 externalLinks.ts)。
/// 站内链接(同源)不拦,iframe 内正常导航;http(s) 以外的 mailto:/tel: 一并
/// 交壳页(opener 的 allow-default-urls 支持)。
pub const EXTERNAL_LINK_SCRIPT: &str = r#"(function () {
  if (window.parent === window) {
    return; // 顶层帧(壳页):不处理,壳页链接走 Rust on_navigation
  }
  var handle = function (e) {
    var target = e.target;
    var anchor = target instanceof Element ? target.closest("a") : null;
    if (!anchor) return;
    var href = anchor.getAttribute("href");
    if (!href) return;
    var url;
    try {
      url = new URL(href, anchor.baseURI);
    } catch (_) {
      return;
    }
    if (url.protocol === "http:" || url.protocol === "https:") {
      if (url.origin === window.location.origin) return; // 站内:iframe 内正常导航
    } else if (url.protocol !== "mailto:" && url.protocol !== "tel:") {
      return; // 其它协议:交给默认行为
    }
    e.preventDefault();
    e.stopPropagation();
    window.parent.postMessage({ type: "open-external", url: url.href }, "*");
  };
  document.addEventListener("click", handle, true);
  document.addEventListener("auxclick", handle, true);
})();"#;

/// 创建主窗口并挂导航拦截 + 页面层外链拦截脚本。
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
    let builder = WebviewWindowBuilder::from_config(app, conf)?
        // 页面层外链拦截(#29/#36):注入所有帧,子帧外链经 postMessage 回壳页
        // (Windows 上 on_navigation / on_new_window 对 iframe 不触发,wry#1593)
        .initialization_script_for_all_frames(EXTERNAL_LINK_SCRIPT)
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
        });
    // 三平台同行(#42,ADR 0003):Windows/Linux 关系统装饰、全自绘窗口控制
    // (菜单条一行内的拖拽区 + 自绘三按钮);macOS 保持 Overlay(config 定义,
    // 系统红绿灯)。不走平台 conf 文件:其合并是 json-patch(RFC 7396),
    // windows 数组整体替换,会丢基础 conf 的窗口几何。
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.decorations(false);
    builder.build()?;
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
        // 壳本地页(prod tauri.localhost / dev devUrl)不被顶层拦截误伤
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
