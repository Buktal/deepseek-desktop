// 主题 payload 守卫的纯逻辑测试。生产路径:useThemeSync 监听 theme-changed
// 事件与 theme_state 快照返回都经 isResolvedTheme 校验后才应用类名;
// 模块顶层不触碰 window/document,纯 node 环境可 import(不变量)。
import { describe, expect, it } from "vitest"

import { isResolvedTheme } from "@/lib/useThemeSync"

describe("isResolvedTheme", () => {
  it("accepts the contract strings (light|dark)", () => {
    expect(isResolvedTheme("light")).toBe(true)
    expect(isResolvedTheme("dark")).toBe(true)
  })

  it("rejects everything else, including the choice string system", () => {
    // "system" 是选择串(tray-theme 契约),不是生效主题——前端不消费、不应用
    expect(isResolvedTheme("system")).toBe(false)
    expect(isResolvedTheme("")).toBe(false)
    expect(isResolvedTheme(undefined)).toBe(false)
    expect(isResolvedTheme(null)).toBe(false)
    expect(isResolvedTheme(42)).toBe(false)
    expect(isResolvedTheme({ resolved: "dark" })).toBe(false)
  })
})
