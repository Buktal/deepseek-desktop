// 全屏本地页上的收容卡片外壳(应用更新卡 / dsh 升级卡共用)。
// 视觉(#20 审核定稿):决策面 = 收容卡片(bg-card + border),与 boot 流程的
// 开放画布区分——升级是用户可执行的动作面板,不是状态仪表。
// 原先 UpdateCard / UpgradeScreen 各持一份相同结构,归并到本组件。

import type { ReactNode } from "react"

export function FullScreenCard({ children }: { children: ReactNode }) {
  return (
    <main className="flex h-screen w-screen items-center justify-center bg-background p-10 text-foreground">
      <div className="flex w-full max-w-md flex-col items-center gap-5 rounded-2xl border border-border bg-card p-10 text-center shadow-sm">
        {children}
      </div>
    </main>
  )
}
