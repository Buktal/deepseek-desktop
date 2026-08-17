// 壳页骨架(M1,#36 + M2,#37):壳菜单条 + dsh iframe 容器 + 浮层挂载点。
// - 壳菜单条:同一组件三平台变体(macOS 28px 拖拽条 / Win+Linux 内容区
//   第一行,ADR 0002,#37);菜单快照与下拉由 M3 填充。
// - iframe:dsh 跨源嵌入(ADR 0001),src 由 Rust 推送的 dsh URL 设置;
//   allow 放行全屏与剪贴板 API(跨源 iframe 的 Web 能力需显式授权)。
// - 浮层挂载点:盖在 iframe 之上(z-20),boot / 升级卡 / 更新卡 / 错误页都
//   渲染在此;无浮层时 pointer-events-none 穿透到 iframe,不挡交互。
// 布局:菜单条不参与互斥(CONTEXT.md「菜单条不参与互斥,任何阶段常驻」),
// 浮层只覆盖 iframe 容器区域。
import type { ReactNode } from "react"

import { MenuBar } from "@/components/shell/MenuBar"
import { cn } from "@/lib/utils"

export function ShellLayout({
  dshUrl,
  children,
}: {
  /** 当前 dsh URL(Rust record_dsh_url 推送,单一事实来源;null = 未就绪) */
  dshUrl: string | null
  /** 浮层内容(boot / 升级卡 / 更新卡 / 错误页;空 = 只显示 iframe) */
  children: ReactNode
}) {
  return (
    <div className="flex h-screen w-screen flex-col bg-background text-foreground">
      {/* 壳菜单条(M2 三平台布局:macOS 28px 拖拽条 / Win+Linux 内容区第一行) */}
      <MenuBar />
      {/* iframe 容器 + 浮层挂载点 */}
      <div className="relative min-h-0 flex-1">
        <iframe
          title="dsh"
          allow="fullscreen; clipboard-read; clipboard-write"
          className="absolute inset-0 h-full w-full"
          src={dshUrl ?? undefined}
        />
        <div className={cn("absolute inset-0 z-20", !children && "pointer-events-none")}>
          {children}
        </div>
      </div>
    </div>
  )
}
