// 主题同步(useThemeSync):App 挂载时启用,把 Rust 下发的生效主题应用到
// <html>.dark —— boot UI 的 Tailwind 变量方案随类切换(见 index.css)。
// Rust 侧为事实源(theme.rs):`theme_state` 快照 + `theme-changed` 事件
// (payload "light"|"dark")单向下发;跟随系统时 OS 主题变化由 Rust 监听并实时重推。
// 初始化全在 effect 内:模块顶层不触碰 window/document(守卫同 useBoot,
// vitest 纯 node 环境可安全 import)。
// 竞态语义与 boot-state 一致:先注册监听再拉快照,两者同源,后到者覆盖。
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useEffect, useState } from "react"

export type ResolvedTheme = "light" | "dark"

/** 生效主题 payload 守卫:只接受契约串("light"|"dark"),未知值不应用到页面。纯函数,可测。 */
export function isResolvedTheme(v: unknown): v is ResolvedTheme {
  return v === "light" || v === "dark"
}

export function useThemeSync(): void {
  // 默认亮色:快照到达前 boot 页按亮色渲染(IPC 毫秒级,首帧后即正确主题)
  const [isDark, setIsDark] = useState(false)

  useEffect(() => {
    let mounted = true
    const unlisteners: UnlistenFn[] = []

    void (async () => {
      try {
        const un = await listen<unknown>("theme-changed", (e) => {
          if (mounted && isResolvedTheme(e.payload)) setIsDark(e.payload === "dark")
        })
        unlisteners.push(un)
        if (mounted) {
          const snap = await invoke<ResolvedTheme>("theme_state")
          if (mounted && isResolvedTheme(snap)) setIsDark(snap === "dark")
        }
      } catch (e) {
        // 监听/快照失败:保持默认亮色,不阻塞启动页(主题是增强,不是硬依赖)
        console.warn("[theme] 主题同步失败,保持亮色", e)
      }
    })()

    return () => {
      mounted = false
      unlisteners.forEach((u) => u())
    }
  }, [])

  useEffect(() => {
    document.documentElement.classList.toggle("dark", isDark)
  }, [isDark])
}
