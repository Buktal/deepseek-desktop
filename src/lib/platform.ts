// 平台探测 + 壳菜单条三平台布局参数(ADR 0002 / #37)。
// 拍板 #28:macOS Overlay——28px 拖拽条与系统红绿灯同行;
// Windows/Linux 保留系统标题栏,菜单条放内容区第一行(36px)。
// 全部为纯函数:模块顶层不触碰 window/navigator,vitest 纯 node 环境可安全
// import(与 useThemeSync 同款不变量)。

export type Platform = "macos" | "windows" | "linux"

/** UA → 平台。未知/缺失一律按 linux 兜底(保守侧:不给 drag region,
 *  与 CC-Switch 的 platform.ts 同款语义)。 */
export function detectPlatform(ua: string): Platform {
  if (/windows/i.test(ua)) return "windows"
  if (/macintosh|mac os x/i.test(ua)) return "macos"
  return "linux"
}

/** 当前平台(渲染期调用;navigator 缺失时按 linux 兜底,测试环境安全)。 */
export function currentPlatform(): Platform {
  return detectPlatform(typeof navigator === "undefined" ? "" : navigator.userAgent)
}

/** drag region 属性策略(ADR 0002 / #28 施工要点):
 *  非 macOS 必须完全不渲染属性——wry 是属性存在性检测,空字符串/"false"
 *  仍会触发(tauri#13440:Linux Wayland 下 drag 触发窗口事件异常)。 */
export type DragRegionProps = { "data-tauri-drag-region": "true" }

export function dragRegionProps(platform: Platform): DragRegionProps | undefined {
  return platform === "macos" ? { "data-tauri-drag-region": "true" } : undefined
}

/** 壳菜单条布局参数(数值为原型 #30 定稿):
 *  - macOS:28px(h-7)拖拽条,左侧 84px 避让系统红绿灯(社区经验值 ~80px,
 *    原型确认);无底部分隔线——Overlay 下系统标题栏为透明覆盖,画线/
 *    背景色块会形成双层标题栏视觉(ADR 0002 坑位)。
 *  - Windows/Linux:36px(h-9)内容区第一行 + 底部分隔线。 */
export interface MenuBarLayout {
  heightClass: string
  paddingClass: string
  borderClass: string
}

export function menuBarLayout(platform: Platform): MenuBarLayout {
  switch (platform) {
    case "macos":
      return { heightClass: "h-7", paddingClass: "pl-[84px]", borderClass: "border-0" }
    case "windows":
    case "linux":
      return { heightClass: "h-9", paddingClass: "px-3", borderClass: "border-b border-border" }
  }
}
