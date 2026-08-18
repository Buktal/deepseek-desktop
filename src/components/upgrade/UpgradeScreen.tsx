// dsh 升级全屏覆盖层(#17,#3 §1 定稿形态 + #32/#40 迁移):四路互斥覆盖层
// 之一(deriveOverlay 编排,F4 优先级 Error > Upgrade > Boot > Update)。
// 升级流水线在途时 dsh 已杀(iframe 无内容),覆盖层即主画面;菜单条不参与
// 互斥,任何阶段常驻。
//
// 视觉(#20 审核定稿 + #30 原型疑点 7 结论):决策面(available/failed)= 收容
// 卡片(shadcn Card)——升级是用户可执行的动作面板;状态面(active)= 开放画布
// 竖排四阶段步进(killing → installing → verifying → starting,原型截图形态,
// 去掉 phase key 调试徽标),安装中叠加确定进度(与 boot 共用 InstallProgress)。
// 覆盖层退场(疑点 6 结论):ready 瞬态由 Rust 立即消费(Reset 紧随 Succeeded),
// 前端无从稳定呈现动画起点,直接切画面,不播 boot 溶解动画。
//
// 状态机由 Rust 侧持有(upgrade.rs 单一事实源),本组件只按 upgrade-state
// 快照/事件渲染:
// - available:发现新版 + 中断影响明示(#3 §4,确认按钮即授权点)+ [立即升级][稍后]
// - active:竖排四阶段步进——killing/verifying 滑动指示,installing 确定进度
//   (InstallProgress 与 boot 共用模拟逻辑,#7);starting 显示「正在重启 dsh…」
// - ready:瞬态(Rust 随即推新 URL),显示「正在打开 dsh…」
// - failed:失败卡(旧版保留 + 恢复服务语义,#3 §3)+ [重试][返回 dsh]
//
// 文案键归 `upgrade.*` + `errors.*`(locale JSON,zh/en 键集一致性由单测守住)。

import { Check, CircleArrowUp, Loader2, RotateCw } from "lucide-react"
import { useTranslation } from "react-i18next"

import { InstallProgress } from "@/components/install/InstallProgress"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { localizeStructuredError, type StructuredError } from "@/lib/error"
import type { InstallStage } from "@/lib/installStage"
import type { DshUpgradePhase, DshUpgradeStatus } from "@/lib/useDshUpgrade"

/** 四阶段步进(疑点 7 定稿形态):键 = Rust phase 串,文案经 upgrade.stage.* 翻译 */
const STAGES: readonly { key: DshUpgradePhase; labelKey: string }[] = [
  { key: "killing", labelKey: "upgrade.stage.killing" },
  { key: "installing", labelKey: "upgrade.stage.installing" },
  { key: "verifying", labelKey: "upgrade.stage.verifying" },
  { key: "starting", labelKey: "upgrade.stage.starting" },
]

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
  stage: InstallStage | null
  error: StructuredError | null
  onConfirm: () => void
  onDismiss: () => void
}) {
  const { t } = useTranslation()
  // 安装类阶段(killing/installing/verifying)共用「正在升级 dsh…」;
  // starting 显示「正在重启 dsh…」(key 缺省时 defaultValue 兜底)
  const installing = phase === "killing" || phase === "installing" || phase === "verifying"
  // #41 上游耦合防线:帧嵌入被新版 dsh 禁止——安装已成功、旧版已被替换,
  // 「当前版本仍可使用/重试」不成立,keepOld 行让位给 errors.UpgradeFrameBlocked
  // 的引导文案(回退预案 = 恢复整窗互斥导航,git 历史可回)
  const frameBlocked =
    error?.kind === "app" && error?.type === "UpgradeFrameBlocked"
  const title = installing
    ? t("upgrade.installing.title")
    : phase === "starting"
      ? t("upgrade.restarting.title")
      : null

  // 决策面(available / failed):收容卡片(shadcn Card,FullScreenCard 退役)
  if (status === "available" || status === "failed") {
    return (
      <main className="flex h-full w-full items-center justify-center bg-background p-10 text-foreground">
        <Card className="flex w-full max-w-md flex-col items-center gap-5 rounded-3xl p-10 text-center shadow-sm">
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
              <div className="flex items-center gap-3">
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
          {status === "failed" && (
            <>
              <CircleArrowUp className="size-9 text-destructive" />
              <h1 className="text-lg font-medium">{t("upgrade.failed.title")}</h1>
              {error ? (
                <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
                  {localizeStructuredError(error, t) || t("errors.unknown")}
                </p>
              ) : null}
              {/* 失败语义(#3 §3):旧版保留(npm 语义)+ 恢复服务([返回 dsh] 先起旧版再导航)。
                   UpgradeFrameBlocked(#41 上游耦合防线)例外:npm 安装已成功、旧版已被替换,
                   「当前版本仍可使用/重试」不成立——引导文案在 errors.UpgradeFrameBlocked
                   (回退预案 = 恢复整窗互斥导航),不再叠加误导性的 keepOld 行 */}
              {!frameBlocked && (
                <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
                  {t("upgrade.failed.keepOld")}
                </p>
              )}
              <div className="flex items-center gap-3">
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
        </Card>
      </main>
    )
  }

  // 状态面(active / ready):开放画布 + 竖排四阶段步进(疑点 7 结论)
  const currentIndex = phase ? STAGES.findIndex((s) => s.key === phase) : -1
  return (
    <main className="flex h-full w-full flex-col items-center justify-center gap-6 bg-background text-foreground">
      {status === "active" ? (
        <>
          {/* 头部:活动图标 + 阶段标题(「正在升级 dsh…」/「正在重启 dsh…」) */}
          <div className="flex items-center gap-4">
            <Loader2 className="size-9 shrink-0 animate-spin text-primary" />
            <div>
              <h1 className="text-lg font-medium">{title}</h1>
              {installing ? (
                <p className="mt-1 text-sm text-muted-foreground">{t("upgrade.installing.hint")}</p>
              ) : null}
            </div>
          </div>
          {/* 四阶段步进:done(勾选)/ current(转圈)/ pending(空圈)三态 */}
          <ol className="flex w-full max-w-lg flex-col gap-2.5 px-6">
            {STAGES.map((s, i) => {
              const state = i < currentIndex ? "done" : i === currentIndex ? "current" : "pending"
              return (
                <li key={s.key} className="flex items-center gap-2.5 text-sm">
                  {state === "done" && (
                    <span
                      aria-hidden
                      className="flex size-4 items-center justify-center rounded-full bg-primary text-primary-foreground"
                    >
                      <Check className="size-2.5" />
                    </span>
                  )}
                  {state === "current" && (
                    <Loader2 className="size-4 shrink-0 animate-spin text-primary" aria-hidden />
                  )}
                  {state === "pending" && (
                    <span
                      aria-hidden
                      className="size-4 shrink-0 rounded-full border-2 border-muted-foreground/30"
                    />
                  )}
                  <span
                    className={
                      state === "pending"
                        ? "text-muted-foreground/50"
                        : state === "current"
                          ? "font-medium"
                          : "text-muted-foreground"
                    }
                  >
                    {t(s.labelKey)}
                  </span>
                </li>
              )
            })}
          </ol>
          {/* 进度:installing 确定进度(阶段 + 百分比),其余阶段滑动指示
              (与 boot 共用 InstallProgress,#7) */}
          <div className="w-full max-w-lg px-6">
            <InstallProgress
              progress={phase === "installing" ? progress : null}
              stage={stage}
              i18nPrefix="upgrade"
            />
          </div>
        </>
      ) : (
        /* ready 瞬态:Rust 随即推新 URL 给壳页(iframe 自动切换),覆盖层直接
           退场(疑点 6:ready 被 Rust 立即消费,无动画起点) */
        <>
          <Loader2 className="size-9 animate-spin text-primary" />
          <h1 className="text-lg font-medium">{t("boot.ready.title")}</h1>
          <div className="w-full max-w-lg px-6">
            <InstallProgress progress={null} stage={null} i18nPrefix="upgrade" />
          </div>
        </>
      )}
    </main>
  )
}
