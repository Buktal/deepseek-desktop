// 错误页:失败原因 + 最近日志 + 重试/退出(外壳界面不展示 Logo)。
// 视觉(#20 审核定稿):错误属于 boot 流程,保持开放画布(非升级卡的收容卡片),
// 居中列 + 结构化信息层次;日志框与文案列同宽,stderr 暗色下用 red-400 保对比。
// error 是未翻译的结构化失败原因(Rust BootError 或 raw 串),渲染时才翻译,
// 语言切换后重新渲染会得到新语言的文案而不是冻结旧串。
// #13:NodeMissing / NodeVersionUnmet 两个 kind 走 Node 引导页(NodeGuideScreen,
// 展示版本要求 + 当前检测结果 + 官网下载),其余错误留本页通用布局——
// 两者共用 retry/quit 结构,重试即重走 boot 流水线(装好 Node 后一键恢复)。
import { useTranslation } from "react-i18next"

import { NodeGuideScreen } from "@/components/boot/NodeGuideScreen"
import { Button } from "@/components/ui/button"
import { describeError, isNodeGuideError, toStructuredError } from "@/lib/error"
import { cn } from "@/lib/utils"
import type { BootLog } from "@/lib/useBoot"

export function ErrorScreen({
  error,
  logs,
  retry,
  quit,
}: {
  error: unknown
  logs: BootLog[]
  retry: () => void
  quit: () => void
}) {
  const { t } = useTranslation()
  // 引导页判定:error 已结构化(toStructuredError 归约幂等),判别与渲染分离
  const structured = toStructuredError(error)
  if (isNodeGuideError(structured)) {
    return <NodeGuideScreen error={structured} retry={retry} quit={quit} />
  }
  const message = describeError(error, t) || t("errors.unknown")
  const tail = logs.slice(-5)
  return (
    <main className="flex h-full w-full flex-col items-center justify-center gap-8 bg-background text-foreground">
      <div className="flex w-full max-w-md flex-col items-center gap-5 text-center">
        <h1 className="text-lg font-medium">{t("error.title")}</h1>
        <p className="text-sm leading-relaxed text-muted-foreground">{message}</p>

        {tail.length > 0 && (
          <div className="max-h-28 w-full overflow-y-auto rounded-xl border border-border bg-muted/60 p-3 text-left font-mono text-xs leading-relaxed text-muted-foreground">
            {tail.map((l, i) => (
              <div
                key={i}
                className={cn("truncate", l.stream === "stderr" && "text-red-500 dark:text-red-400")}
              >
                {l.line}
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="flex items-center gap-3">
        <Button size="lg" onClick={retry}>
          {t("common.retry")}
        </Button>
        <Button variant="ghost" size="lg" onClick={quit}>
          {t("common.quit")}
        </Button>
      </div>
    </main>
  )
}
