// dsh 升级卡片(#17,#3 §1 定稿形态):全屏卡片,外壳本地页上展示。
// 形态与应用升级卡同款:不用 Popover(本外壳无常驻可锚定页面),不用模态弹窗
// (升级流程含分钟级安装进度,卡片提供「稍后」,不强制决策)。
//
// 状态机由 Rust 侧持有(upgrade.rs 单一事实源),本组件只按 upgrade-state
// 快照/事件渲染:
// - available:发现新版 + 中断影响明示(#3 §4,确认按钮即授权点)+ [立即升级][稍后]
// - active:流水线进度——killing/installing/verifying 显示「正在升级 dsh…」,
//   installing 复用 InstallProgress(与 boot 共用模拟逻辑,#7);starting 显示
//   「正在重启 dsh…」
// - ready:瞬态(Rust 随即导航回 dsh 页),显示「正在打开 dsh…」
// - failed:失败卡(旧版保留 + 恢复服务语义,#3 §3)+ [重试][返回 dsh];
//   错误经 localizeStructuredError 渲染时翻译(errors.<kind> 键)
//
// 文案键归 `upgrade.*` + `errors.*`(locale JSON,zh/en 键集一致性由单测守住)。

import { CircleArrowUp, Loader2, RotateCw } from "lucide-react"
import { useTranslation } from "react-i18next"

import { InstallProgress } from "@/components/install/InstallProgress"
import { Button } from "@/components/ui/button"
import { localizeStructuredError, type StructuredError } from "@/lib/error"
import type { DshUpgradePhase, DshUpgradeStatus } from "@/lib/useDshUpgrade"

/** 不确定进度条(滑动指示):killing/verifying 阶段与 ready 过渡用。 */
function IndeterminateBar() {
  return (
    <div className="h-1.5 w-72 overflow-hidden rounded-full bg-muted" role="progressbar">
      <div className="h-full w-1/3 animate-loading-slide rounded-full bg-indigo-500" />
    </div>
  )
}

export function UpgradeScreen({
  status,
  version,
  currentVersion,
  phase,
  progress,
  stage,
  error,
  onConfirm,
  onDismiss,
}: {
  status: DshUpgradeStatus
  version: string | null
  currentVersion: string | null
  phase: DshUpgradePhase | null
  progress: number | null
  stage: string | null
  error: StructuredError | null
  onConfirm: () => void
  onDismiss: () => void
}) {
  const { t } = useTranslation()
  // 安装类阶段(killing/installing/verifying)共用「正在升级 dsh…」;
  // starting 显示「正在重启 dsh…」(key 缺省时 defaultValue 兜底)
  const installing = phase === "killing" || phase === "installing" || phase === "verifying"
  const title = installing
    ? t("upgrade.installing.title")
    : phase === "starting"
      ? t("upgrade.restarting.title")
      : null

  return (
    <main className="flex h-screen w-screen flex-col items-center justify-center gap-7 bg-background text-foreground">
      <div className="flex w-full max-w-md flex-col items-center gap-4 text-center">
        {status === "available" && (
          <>
            <CircleArrowUp className="size-9 text-primary" />
            <h1 className="text-lg font-medium">{t("upgrade.available.title")}</h1>
            <p className="text-sm text-muted-foreground">
              {t("upgrade.available.hint", { current: currentVersion, version })}
            </p>
            {/* 中断影响明示(#3 §4):确认按钮即授权点,按下即杀 dsh,不二次确认 */}
            <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
              {t("upgrade.available.impact")}
            </p>
            <div className="flex items-center gap-3 pt-1">
              <Button size="lg" onClick={onConfirm}>
                <CircleArrowUp />
                {t("upgrade.now")}
              </Button>
              <Button variant="ghost" size="lg" onClick={onDismiss}>
                {t("upgrade.later")}
              </Button>
            </div>
          </>
        )}

        {status === "active" && title && (
          <>
            <Loader2 className="size-9 animate-spin text-primary" />
            <h1 className="text-lg font-medium">{title}</h1>
            {installing ? (
              <p className="text-sm text-muted-foreground">{t("upgrade.installing.hint")}</p>
            ) : null}
            {/* 安装中:确定进度(与 boot 共用 InstallProgress,#7);其余阶段不确定进度 */}
            {phase === "installing" && progress !== null ? (
              <InstallProgress progress={progress} stage={stage} i18nPrefix="upgrade" />
            ) : (
              <IndeterminateBar />
            )}
          </>
        )}

        {status === "ready" && (
          <>
            <Loader2 className="size-9 animate-spin text-primary" />
            {/* 瞬态:Rust 随即导航回 dsh 页(新端口 URL);复用 boot 的过渡文案 */}
            <h1 className="text-lg font-medium">{t("boot.ready.title")}</h1>
            <IndeterminateBar />
          </>
        )}

        {status === "failed" && (
          <>
            <CircleArrowUp className="size-9 text-destructive" />
            <h1 className="text-lg font-medium">{t("upgrade.failed.title")}</h1>
            {error ? (
              <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
                {localizeStructuredError(error, t) || t("errors.unknown")}
              </p>
            ) : null}
            {/* 失败语义(#3 §3):旧版保留(npm 语义)+ 恢复服务([返回 dsh] 先起旧版再导航) */}
            <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
              {t("upgrade.failed.keepOld")}
            </p>
            <div className="flex items-center gap-3 pt-1">
              <Button size="lg" onClick={onConfirm}>
                <RotateCw />
                {t("common.retry")}
              </Button>
              <Button variant="ghost" size="lg" onClick={onDismiss}>
                {t("upgrade.failed.back")}
              </Button>
            </div>
          </>
        )}
      </div>
    </main>
  )
}
