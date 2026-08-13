// 错误页:失败原因 + 最近日志 + 重试/退出(外壳界面不展示 Logo)
// error 是未翻译的结构化失败原因(Rust BootError 或 raw 串),渲染时才翻译,
// 语言切换后重新渲染会得到新语言的文案而不是冻结旧串。
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { describeError } from "@/lib/error"
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
  const message = describeError(error, t) || t("errors.unknown")
  const tail = logs.slice(-5)
  return (
    <main className="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-background text-foreground">
      <div className="max-w-md text-center">
        <h1 className="text-lg font-medium">{t("error.title")}</h1>
        <p className="mt-1 text-sm leading-relaxed text-muted-foreground">{message}</p>
      </div>

      {tail.length > 0 && (
        <div className="max-h-24 w-80 overflow-y-auto rounded-lg bg-muted/50 p-3 font-mono text-xs leading-relaxed text-muted-foreground">
          {tail.map((l, i) => (
            <div key={i} className={cn("truncate", l.stream === "stderr" && "text-red-500")}>
              {l.line}
            </div>
          ))}
        </div>
      )}

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
