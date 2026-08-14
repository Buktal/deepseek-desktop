// 主题同步(useThemeSync):App 挂载时启用,把 Rust 下发的生效主题应用到
// <html>.dark —— boot UI 的 Tailwind 变量方案随类切换(见 index.css)。
// Rust 侧为事实源(theme.rs):`theme_state` 快照 + `theme-changed` 事件
// (payload "light"|"dark")单向下发;跟随系统时 OS 主题变化由 Rust 监听并实时重推。
// 初始化全在 effect 内:模块顶层不触碰 window/document(守卫同 useBoot,
// vitest 纯 node 环境可安全 import)。
// 同步骨架(先注册监听再拉快照,两者同源,后到者覆盖)走 useRustStateSync;
// 生效主题守卫(isResolvedTheme)在 apply 处统一做:事件与快照同一条校验路径。
import { invoke } from "@tauri-apps/api/core"
import { useEffect, useState } from "react"

import { useRustStateSync } from "@/lib/useRustStateSync"

export type ResolvedTheme = "light" | "dark"

/** 生效主题 payload 守卫:只接受契约串("light"|"dark"),未知值不应用到页面。纯函数,可测。 */
export function isResolvedTheme(v: unknown): v is ResolvedTheme {
  return v === "light" || v === "dark"
}

export function useThemeSync(): void {
  // 默认亮色:快照到达前 boot 页按亮色渲染(IPC 毫秒级,首帧后即正确主题)
  const [isDark, setIsDark] = useState(false)

  useRustStateSync({
    event: "theme-changed",
    snapshot: () => invoke<ResolvedTheme>("theme_state"),
    apply: (view) => {
      if (isResolvedTheme(view)) setIsDark(view === "dark")
    },
    // 监听/快照失败:保持默认亮色,不阻塞启动页(主题是增强,不是硬依赖)
    onError: (e) => console.warn("[theme] 主题同步失败,保持亮色", e),
  })

  useEffect(() => {
    document.documentElement.classList.toggle("dark", isDark)
  }, [isDark])
}
