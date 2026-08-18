// 平台探测 + 壳菜单条三平台布局参数(#42 三平台同行,ADR 0003)。
// macOS Overlay——28px 拖拽条与系统红绿灯同行(左侧 84px 避让);
// Windows/Linux 无边框全自绘——36px 一行:左菜单按钮 + 中部拖拽区 +
// 右侧自绘窗口控制贴右缘(decorations 由 navigation.rs 按平台关闭)。
// 全部为纯函数:模块顶层不触碰 window/navigator,vitest 纯 node 环境可安全
// import(与 useThemeSync 同款不变量)。

export type Platform = "macos" | "windows" | "linux"

/** UA → 平台。未知/缺失一律按 linux 兜底(兜底侧与 Linux 真机同形:
 * 自绘控制条 + drag region;真机 UA 恒存在,兜底只影响测试/异常环境)。 */
export function detectPlatform(ua: string): Platform {
  if (/windows/i.test(ua)) return "windows"
  if (/macintosh|mac os x/i.test(ua)) return "macos"
  return "linux"
}

/** 当前平台(渲染期调用;navigator 缺失时按 linux 兜底,测试环境安全)。 */
export function currentPlatform(): Platform {
  return detectPlatform(typeof navigator === "undefined" ? "" : navigator.userAgent)
}

/** drag region 属性策略(ADR 0003:三平台菜单条均为拖拽区;drag.js 对
 *  BUTTON 等可点击元素自动豁免,双击最大化随 drag 内建)。
 *  渲染侧约束保留:平台/场景禁用时必须完全不渲染属性——wry 是属性
 *  存在性检测,空字符串/"false" 仍会触发(tauri#13440)。 */
export type DragRegionProps = { "data-tauri-drag-region": "true" }

export function dragRegionProps(): DragRegionProps {
  return { "data-tauri-drag-region": "true" }
}

/** 壳菜单条布局参数(macOS 数值为原型 #30 定稿;Windows/Linux #42 同行化):
 *  - macOS:28px(h-7)与红绿灯同行,左侧 84px 避让系统红绿灯
 *    (trafficLightPosition 已在 tauri.conf.json 定位);无底部分隔线——
 *    Overlay 下系统标题栏为透明覆盖,画线/背景色块会形成双层标题栏视觉。
 *  - Windows/Linux:36px(h-9)一行,左内边距常规、右零内边距(自绘窗口
 *    控制贴右缘,同系统按钮布局)+ 底部分隔线。 */
export interface MenuBarLayout {
  heightClass: string
  paddingClass: string
  borderClass: string
  /** 行内是否渲染自绘窗口控制(macOS 用系统红绿灯,不渲染) */
  windowControls: boolean
}

export function menuBarLayout(platform: Platform): MenuBarLayout {
  switch (platform) {
    case "macos":
      return {
        heightClass: "h-7",
        paddingClass: "pl-[84px]",
        borderClass: "border-0",
        windowControls: false,
      }
    case "windows":
    case "linux":
      return {
        heightClass: "h-9",
        paddingClass: "pl-3 pr-0",
        borderClass: "border-b border-border",
        windowControls: true,
      }
  }
}
