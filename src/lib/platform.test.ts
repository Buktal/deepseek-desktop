// platform 纯函数模块测试(#37;#42 三平台同行改版,ADR 0003)。守住的契约:
// - detectPlatform:UA → 平台,Windows 优先,未知兜底 linux;
// - dragRegionProps:三平台菜单条均为拖拽区,恒渲染属性(禁用侧约束保留:
//   必须完全不渲染属性,wry 是属性存在性检测);
// - menuBarLayout:三平台一行化布局参数(macOS 沿用原型 #30 定稿,
//   Windows/Linux 改自绘控制贴右缘)。
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
    const ua =
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36"
    expect(detectPlatform(ua)).toBe("linux")
  })

  it("falls back to linux for empty or unknown UA", () => {
    // 兜底语义:未知平台按 linux 处理——自绘控制条 + drag region,
    // 与 Linux 真机同形(真机 UA 恒存在,兜底只影响测试/异常环境)
    expect(detectPlatform("")).toBe("linux")
    expect(detectPlatform("Mozilla/5.0 (X11; FreeBSD)")).toBe("linux")
  })
})

describe("dragRegionProps (ADR 0003:三平台菜单条均为拖拽区)", () => {
  it("恒渲染 data-tauri-drag-region;禁用必须完全不渲染属性——空字符串/\"false\" 仍会被 wry 检测为存在", () => {
    expect(dragRegionProps()).toEqual({ "data-tauri-drag-region": "true" })
  })
})

describe("menuBarLayout (macOS 数值沿用原型 #30;Windows/Linux #42 同行化)", () => {
  it("macOS:28px 与红绿灯同行 + 左侧 84px 避让 + 无分隔线(防双层标题栏视觉)+ 不自绘控制", () => {
    expect(menuBarLayout("macos")).toEqual({
      heightClass: "h-7",
      paddingClass: "pl-[84px]",
      borderClass: "border-0",
      windowControls: false,
    })
  })

  it("Windows/Linux:36px 一行 + 左内边距、右零内边距(自绘控制贴右缘)+ 底部分隔线", () => {
    expect(menuBarLayout("windows")).toEqual({
      heightClass: "h-9",
      paddingClass: "pl-3 pr-0",
      borderClass: "border-b border-border",
      windowControls: true,
    })
    expect(menuBarLayout("linux")).toEqual(menuBarLayout("windows"))
  })
})
