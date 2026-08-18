// 应用更新进度浮层(#31 拍板 / #39 施工):右下角非模态浮层——确认弹窗关闭后
// 下载中显示百分比 + Progress(update-state Downloading 数据复用),完成后
// 变 [立即重启][稍后]。下载不打断使用 dsh——非模态,不盖整屏、不挡交互。
//
// 浮层可关性(原型疑点 4 结论):下载中无关闭入口——update.rs 状态机 Dismissed
// 只对 Available/Ready/Failed 生效(下载中忽略,防「关了但下载还在后台继续」
// 的割裂状态);完成后 [稍后] = dismiss → 状态归 Idle,浮层消失。
// 与 toast 同角(右下)叠放(疑点 8 结论):sonner 无内建避让,接受叠放——
// 两者共存场景罕见(toast 短暂、浮层低频),自定义避让逻辑属过度设计。
import { RotateCw } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Progress } from "@/components/ui/progress"
import { Button } from "@/components/ui/button"
import { updatePercent } from "@/lib/updateShare"

export function UpdateFloat({
  status,
  downloadedBytes,
  totalBytes,
  onRestart,
  onDismiss,
}: {
  /** "downloading"(下载中) | "ready"(已下载,等待重启) */
  status: "downloading" | "ready"
  downloadedBytes: number
  totalBytes: number
  onRestart: () => void
  onDismiss: () => void
}) {
  const { t } = useTranslation()
  const pct = updatePercent(downloadedBytes, totalBytes)

  return (
    <div className="fixed right-4 bottom-4 z-30 w-80 animate-in fade-in slide-in-from-bottom-3 motion-reduce:animate-none rounded-3xl border bg-popover p-4 text-popover-foreground shadow-lg">
      {status === "downloading" ? (
        <>
          <div className="flex items-center justify-between gap-2">
            <p className="text-sm font-medium">{t("update.downloading")}</p>
            <span className="text-sm text-muted-foreground tabular-nums">
              {pct !== null ? t("update.downloaded", { pct }) : t("update.pleaseWait")}
            </span>
          </div>
          <Progress value={pct} className="mt-2.5" />
        </>
      ) : (
        <>
          <p className="text-sm font-medium">{t("update.ready")}</p>
          {/* 中断影响明示(#3 §4):重启按钮即授权点,文案必须说清语义 */}
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            {t("update.restartToInstall")}
          </p>
          <div className="mt-3 flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={onDismiss}>
              {t("update.later")}
            </Button>
            <Button size="sm" onClick={onRestart}>
              <RotateCw />
              {t("update.restartNow")}
            </Button>
          </div>
        </>
      )}
    </div>
  )
}
