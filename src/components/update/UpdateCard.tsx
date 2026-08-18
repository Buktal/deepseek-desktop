// 应用自身升级卡片(#5):壳页浮层上的升级卡片,盖在 dsh iframe 之上(壳页
// 常驻,#36)。形态定稿(#3 §1):升级主交互 = 卡片收容面板,不用 Popover——
// 本外壳窗口没有可锚定 Popover 的常驻页面;不用模态弹窗——升级流程含分钟级
// 安装进度,卡片提供「稍后」,不强制决策。
// 视觉(#20 审核定稿):决策面 = 收容卡片(shadcn Card,M5 迁移后 FullScreenCard
// 退役),与 boot 流程的开放画布区分——升级是用户可执行的动作面板,不是状态
// 仪表。
//
// #39(#31 拍板):下载中/完成不再用整屏卡片打断——下载进度改为右下角非模态
// 浮层 UpdateFloat(下载不打断使用 dsh),本卡片只承载 available(发现新版
// [立即更新])与 failed(失败降级 GitHub 手动下载)两个决策面。
//
// 状态机(available/downloading/ready/failed)由 Rust 侧持有(update.rs 单一
// 事实源),本组件只按 `update-state` 快照/事件渲染对应卡片体;错误经
// localizeStructuredError 渲染时翻译,失败降级 GitHub 手动下载(O_CC_One 同款)。
// 文案键归 `update.*`(locale JSON,zh/en 键集一致性由单测守住)。

import { CircleArrowUp, ExternalLink } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { localizeStructuredError, type StructuredError } from "@/lib/error"
import { summarizeReleaseNotes } from "@/lib/releaseNotes"
import type { UpdateStatus } from "@/lib/useUpdateCheck"

export function UpdateCard({
  status,
  version,
  currentVersion,
  notes,
  error,
  onApply,
  onDismiss,
  onOpenReleases,
}: {
  status: UpdateStatus
  version: string | null
  currentVersion: string | null
  notes: string | null
  error: StructuredError | null
  onApply: () => void
  onDismiss: () => void
  onOpenReleases: () => void
}) {
  const { t } = useTranslation()

  return (
    <main className="flex h-full w-full items-center justify-center bg-background p-10 text-foreground">
      <Card className="flex w-full max-w-md flex-col items-center gap-5 rounded-3xl p-10 text-center shadow-sm">
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
              {summarizeReleaseNotes(notes)}
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

      {status === "failed" && (
        <>
          <CircleArrowUp className="size-9 text-destructive" />
          <h1 className="text-lg font-medium">{t("update.failed")}</h1>
          {error ? (
            <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
              {localizeStructuredError(error, t) || t("errors.unknown")}
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
      </Card>
    </main>
  )
}
