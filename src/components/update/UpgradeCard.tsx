// 应用自身升级卡片(#5):全屏本地页上的升级卡片,外壳窗口内展示。
// 形态定稿(#3 §1):升级主交互 = 外壳本地页上的全屏升级卡片,不用 Popover——
// 本外壳窗口没有可锚定 Popover 的常驻页面(窗口几乎总在 dsh 页,由 Rust 导航
// 回本地页呈现);不用模态弹窗——升级流程含分钟级安装进度,卡片提供「稍后」,
// 不强制决策。
// 视觉(#20 审核定稿):决策面 = 收容卡片(bg-card + border),与 boot 流程的
// 开放画布区分——升级是用户可执行的动作面板,不是状态仪表。
//
// 状态机(available/downloading/ready/failed)由 Rust 侧持有(update.rs 单一
// 事实源),本组件只按 `update-state` 快照/事件渲染对应卡片体;错误经
// localizeStructuredError 渲染时翻译,失败降级 GitHub 手动下载(O_CC_One 同款)。
// 文案键归 `update.*`(locale JSON,zh/en 键集一致性由单测守住)。

import {
  CircleArrowUp,
  ExternalLink,
  Loader2,
  PartyPopper,
  RotateCw,
} from "lucide-react"
import { useTranslation } from "react-i18next"

import { ProgressRail } from "@/components/shell/ProgressRail"
import { Button } from "@/components/ui/button"
import { localizeStructuredError, type StructuredError } from "@/lib/error"
import { updatePercent, type UpdateStatus } from "@/lib/useUpdateCheck"

const NOTES_PREVIEW_LINES = 5

/** 下载中卡片体:确定进度(percent 非 null)或不确定进度(total 未知,「请稍候」)。 */
function DownloadingBody({
  downloadedBytes,
  totalBytes,
  t,
}: {
  downloadedBytes: number
  totalBytes: number
  t: ReturnType<typeof useTranslation>["t"]
}) {
  const pct = updatePercent(downloadedBytes, totalBytes)
  return (
    <>
      <Loader2 className="size-9 animate-spin text-primary" />
      <h1 className="text-lg font-medium">{t("update.downloading")}</h1>
      <p className="text-sm text-muted-foreground">
        {pct !== null ? t("update.downloaded", { pct }) : t("update.pleaseWait")}
      </p>
      <ProgressRail value={pct} />
    </>
  )
}

/** 提炼 release notes:去 markdown 记号与注释行,保留前几行。照搬 O_CC_One。 */
function summarizeNotes(notes: string): string {
  return notes
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith("<!--"))
    .slice(0, NOTES_PREVIEW_LINES)
    .map((l) =>
      l
        .replace(/^[-*+]\s+/, "")
        .replace(/^#+\s*/, "")
        .replace(/[*_`]/g, "")
        .trim(),
    )
    .join("\n")
}

export function UpgradeCard({
  status,
  version,
  currentVersion,
  notes,
  error,
  downloadedBytes,
  totalBytes,
  onApply,
  onRestart,
  onDismiss,
  onOpenReleases,
}: {
  status: UpdateStatus
  version: string | null
  currentVersion: string | null
  notes: string | null
  error: StructuredError | null
  downloadedBytes: number
  totalBytes: number
  onApply: () => void
  onRestart: () => void
  onDismiss: () => void
  onOpenReleases: () => void
}) {
  const { t } = useTranslation()

  return (
    <main className="flex h-screen w-screen items-center justify-center bg-background p-10 text-foreground">
      <div className="flex w-full max-w-md flex-col items-center gap-5 rounded-2xl border border-border bg-card p-10 text-center shadow-sm">
        {status === "available" && (
          <>
            <CircleArrowUp className="size-9 text-primary" />
            <h1 className="text-lg font-medium">
              {t("update.available.found", { version })}
            </h1>
            {currentVersion ? (
              <p className="text-sm text-muted-foreground">
                {t("update.current", { version: currentVersion })}
              </p>
            ) : null}
            {notes ? (
              <p className="max-h-28 max-w-sm overflow-y-auto text-sm leading-relaxed whitespace-pre-line break-words text-muted-foreground">
                {summarizeNotes(notes)}
              </p>
            ) : null}
            <div className="flex items-center gap-3">
              <Button size="lg" onClick={onApply}>
                <CircleArrowUp />
                {t("update.updateNow")}
              </Button>
              <Button variant="ghost" size="lg" onClick={onDismiss}>
                {t("update.later")}
              </Button>
            </div>
          </>
        )}

        {status === "downloading" && (
          <DownloadingBody
            downloadedBytes={downloadedBytes}
            totalBytes={totalBytes}
            t={t}
          />
        )}

        {status === "ready" && (
          <>
            <PartyPopper className="size-9 text-primary" />
            <h1 className="text-lg font-medium">{t("update.ready")}</h1>
            {/* 中断影响明示(#3 §4):重启按钮即授权点,文案必须说清语义 */}
            <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
              {t("update.restartToInstall")}
            </p>
            <div className="flex items-center gap-3">
              <Button size="lg" onClick={onRestart}>
                <RotateCw />
                {t("update.restartNow")}
              </Button>
              <Button variant="ghost" size="lg" onClick={onDismiss}>
                {t("update.later")}
              </Button>
            </div>
          </>
        )}

        {status === "failed" && (
          <>
            <CircleArrowUp className="size-9 text-destructive" />
            <h1 className="text-lg font-medium">{t("update.failed")}</h1>
            {error ? (
              <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
                {localizeStructuredError(error, t)}
              </p>
            ) : null}
            <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
              {t("update.manualHint")}
            </p>
            <div className="flex items-center gap-3">
              <Button variant="outline" size="lg" onClick={onOpenReleases}>
                <ExternalLink />
                {t("update.openGithub")}
              </Button>
              <Button variant="ghost" size="lg" onClick={onDismiss}>
                {t("common.close")}
              </Button>
            </div>
          </>
        )}
      </div>
    </main>
  )
}
