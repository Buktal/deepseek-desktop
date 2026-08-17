// platform 纯函数模块测试(#37)。守住的契约:
// - detectPlatform:UA → 平台,Windows 优先,未知兜底 linux;
// - dragRegionProps:非 macOS 返回 undefined(完全不渲染属性,ADR 0002);
// - menuBarLayout:三平台布局参数,数值与原型 #30 定稿一致。
import { describe, expect, it } from "vitest"

import { detectPlatform, dragRegionProps, menuBarLayout } from "@/lib/platform"

describe("detectPlatform", () => {
  it("detects macOS from WKWebView UA", () => {
    const ua =
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)"
    expect(detectPlatform(ua)).toBe("macos")
  })

  it("detects Windows from WebView2 UA", () => {
    const ua =
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
    expect(detectPlatform(ua)).toBe("windows")
  })

  it("detects Linux from WebKitGTK UA", () => {
    const ua = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
    expect(detectPlatform(ua)).toBe("linux")
  })

  it("falls back to linux for empty or unknown UA", () => {
    // 兜底语义:未知平台按保守侧(linux)处理,不给 drag region
    expect(detectPlatform("")).toBe("linux")
    expect(detectPlatform("Mozilla/5.0 (X11; FreeBSD)")).toBe("linux")
  })
})

describe("dragRegionProps (ADR 0002:非 macOS 完全不渲染属性)", () => {
  it("macOS 渲染 data-tauri-drag-region", () => {
    expect(dragRegionProps("macos")).toEqual({ "data-tauri-drag-region": "true" })
  })

  it("Windows/Linux 返回 undefined——空字符串/\"false\" 仍会被 wry 检测为存在", () => {
    expect(dragRegionProps("windows")).toBeUndefined()
    expect(dragRegionProps("linux")).toBeUndefined()
  })
})

describe("menuBarLayout (数值来自原型 #30 定稿)", () => {
  it("macOS:28px 拖拽条 + 左侧 84px 避让红绿灯 + 无分隔线(防双层标题栏视觉)", () => {
    expect(menuBarLayout("macos")).toEqual({
      heightClass: "h-7",
      paddingClass: "pl-[84px]",
      borderClass: "border-0",
    })
  })

  it("Windows/Linux:36px 内容区第一行 + 常规内边距 + 底部分隔线", () => {
    expect(menuBarLayout("windows")).toEqual({
      heightClass: "h-9",
      paddingClass: "px-3",
      borderClass: "border-b border-border",
    })
    expect(menuBarLayout("linux")).toEqual(menuBarLayout("windows"))
  })
})
