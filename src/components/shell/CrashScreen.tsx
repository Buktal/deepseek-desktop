// dsh 意外退出全屏覆盖层(#31 场景 6 / #32 拍板 / #40 施工):reaper 检测到
// dsh 意外退出(非退出流程 / 非升级流水线)→ dsh-exited 事件 → 本覆盖层。
// dsh 已死、iframe 无内容,弹小窗无意义——全屏覆盖层即主画面,形态与
// ErrorScreen 同款(居中列 + [重试][退出]);[重试] = 重跑 boot 流水线
// (Rust 侧 boot_start 对 Ready + dsh 已死放行)。
// 退出码仅诊断展示(与 boot 错误页的「最近日志」同属排查上下文);数据保存
// 在本机,不受影响(与升级中断影响文案同源语义)。

import { CircleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"

export function CrashScreen({
  exitCode,
  retry,
  quit,
}: {
  /** dsh 进程退出码(null = 未知,如句柄异常) */
  exitCode: number | null
  /** [重试]:重跑 boot 流水线 */
  retry: () => void
  /** [退出]:退出应用 */
  quit: () => void
}) {
  const { t } = useTranslation()
  return (
    <main className="flex h-full w-full flex-col items-center justify-center gap-8 bg-background text-foreground">
      <CircleAlert className="size-9 text-destructive" aria-hidden />
      <div className="flex w-full max-w-md flex-col items-center gap-5 text-center">
        <h1 className="text-lg font-medium">{t("crash.title")}</h1>
        <p className="text-sm leading-relaxed text-muted-foreground">
          {exitCode !== null ? t("crash.message.code", { exitCode }) : t("crash.message")}
        </p>
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
