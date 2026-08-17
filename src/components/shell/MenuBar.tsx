// 壳菜单条(M2,#37):同一组件三平台变体(ADR 0002 / 拍板 #28)。
// - macOS:28px 拖拽条与系统红绿灯同行——data-tauri-drag-region 整行可拖
//   (drag.js 对 BUTTON 等可点击元素自动豁免,单击/双击最大化随 drag 内建),
//   左侧 84px 避让红绿灯(trafficLightPosition 已在 tauri.conf.json 定位);
//   无背景色块与分隔线(Overlay 下系统标题栏为透明覆盖,画了即双层视觉)。
// - Windows/Linux:系统标题栏下方内容区第一行,无 drag region(平台模块
//   返回 undefined,完全不渲染属性——wry 是属性存在性检测)。
// 菜单按钮为占位:菜单快照与下拉由 M3 填充(与托盘同源,M2 只做布局)。
import { MenuIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
import { currentPlatform, dragRegionProps, menuBarLayout } from "@/lib/platform"
import { cn } from "@/lib/utils"

export function MenuBar() {
  const platform = currentPlatform()
  const layout = menuBarLayout(platform)
  return (
    <div
      {...dragRegionProps(platform)}
      className={cn(
        "flex shrink-0 items-center",
        layout.heightClass,
        layout.paddingClass,
        layout.borderClass,
      )}
    >
      {/* 菜单按钮占位(M3 以 MenuSnapshot 快照渲染替换,含文案) */}
      <Button variant="ghost" size="sm" className="h-6 gap-1.5 px-2 text-xs">
        <MenuIcon className="size-3.5" />
        菜单
      </Button>
    </div>
  )
}
