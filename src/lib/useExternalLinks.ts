// 页面层外链拦截的消息接收端(#29/#36):dsh iframe 内命中外链 →
// window.parent.postMessage({type:"open-external",url}) → 本钩子解析校验后
// 经 tauri-plugin-opener 开系统浏览器。Rust 侧注入脚本见 navigation.rs 的
// EXTERNAL_LINK_SCRIPT;解析校验为纯函数 parseExternalLinkMessage(可测)。
// 顶层帧(壳页自身)链接不经此路径——仍走 Rust on_navigation(顶层导航拦截保留)。
import { openUrl } from "@tauri-apps/plugin-opener"
import { useEffect } from "react"

import { parseExternalLinkMessage } from "@/lib/externalLinks"

export function useExternalLinks(): void {
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const url = parseExternalLinkMessage(e.data)
      if (url) void openUrl(url).catch(() => {})
    }
    window.addEventListener("message", onMessage)
    return () => window.removeEventListener("message", onMessage)
  }, [])
}
