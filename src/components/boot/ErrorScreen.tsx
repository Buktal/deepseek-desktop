// 错误页:失败原因 + 最近日志 + 重试/退出
import { Logo } from "@/components/Logo"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type { BootLog } from "@/lib/useBoot"

export function ErrorScreen({
  message,
  logs,
  retry,
  quit,
}: {
  message: string
  logs: BootLog[]
  retry: () => void
  quit: () => void
}) {
  const tail = logs.slice(-5)
  return (
    <main className="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-background text-foreground">
      <Logo size={72} />

      <div className="max-w-md text-center">
        <h1 className="text-lg font-medium">启动失败</h1>
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
          重试
        </Button>
        <Button variant="ghost" size="lg" onClick={quit}>
          退出
        </Button>
      </div>
    </main>
  )
}
