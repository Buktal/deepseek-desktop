// 自绘窗口控制(#42,ADR 0003):Windows/Linux 无边框窗口的最小化/最大化/
// 关闭三按钮,贴壳菜单条右缘(仅这两个平台渲染,macOS 用系统红绿灯;
// 渲染与否由 menuBarLayout().windowControls 决定)。
// - 关闭走 close():先发 closeRequested,由 Rust 侧关闭行为拦截接管
//   (与托盘/菜单同源,尊重 close_behavior 设置)。
// - 最大化图标随窗口状态切换:挂载时 isMaximized() 查询 + onResized 重查。
// - 按钮用原生 button 而非 shadcn Button:系统标题按钮是窗口 chrome
//   (46px 宽、占满行高、无圆角无焦点环),套不进任何现成 variant,
//   整体覆写 cva 反而失真;尺寸对齐 Windows 系统按钮,关闭 hover 用
//   系统惯例红(e81123),亮暗主题同为白图标。
import { getCurrentWindow } from "@tauri-apps/api/window"
import { Copy, Minus, Square, X } from "lucide-react"
import { useEffect, useState } from "react"

import { cn } from "@/lib/utils"

/** 三按钮公共形态:占满菜单条高度、贴右缘、无圆角(系统标题按钮样式)。 */
const controlButton =
  "inline-flex h-full w-[46px] shrink-0 items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"

export function WindowControls() {
  const win = getCurrentWindow()
  const [maximized, setMaximized] = useState(false)
  useEffect(() => {
    const unlisten = win.onResized(async () => setMaximized(await win.isMaximized()))
    win.isMaximized().then(setMaximized)
    return () => {
      unlisten.then((dispose) => dispose())
    }
  }, [win])
  return (
    <div className="ml-auto flex h-full self-stretch">
      <button
        aria-label="最小化"
        type="button"
        className={controlButton}
        onClick={() => win.minimize()}
      >
        <Minus className="size-3.5" />
      </button>
      <button
        aria-label={maximized ? "还原" : "最大化"}
        type="button"
        className={controlButton}
        onClick={() => win.toggleMaximize()}
      >
        {maximized ? <Copy className="size-3" /> : <Square className="size-3" />}
      </button>
      <button
        aria-label="关闭"
        type="button"
        className={cn(controlButton, "hover:bg-[#e81123] hover:text-white")}
        onClick={() => win.close()}
      >
        <X className="size-4" />
      </button>
    </div>
  )
}
