// 启动页(idle / checking / installing / starting / ready 共用):「启动仪表」布局。
// 设计(#20 审核定稿):圆环仪表在左、阶段读数在右的横排仪表行——外壳是启动仪表
// 而非品牌页(无 Logo,品牌由托盘承担),横排让 920px 宽的窗口不被竖排堆叠浪费。
// 仪表环 = 静止刻度环(primary/10)+ 自转活动弧(primary),就绪退出时外层收缩
// 收敛(#4,缩放在外层 wrapper,自转在内层元素,无 transform 冲突)。
// 进度轨与阶段文案对齐在右侧读数列下:安装中显示确定进度(InstallProgress,
// 阶段 + 百分比),其余阶段不确定进度(滑动指示,不显示假百分比);全程显示耗时
// (boot.elapsed.* 键)作右下仪表读数,`aria-live="off"` 每秒不打扰读屏。
// #13:checking 阶段携带 nodeVersion 时显示检测结果(「检测到 Node.js vX」)。
import { Check } from "lucide-react"
import { useTranslation } from "react-i18next"

import { InstallProgress } from "@/components/install/InstallProgress"
import { ProgressRail } from "@/components/shell/ProgressRail"
import { formatElapsed } from "@/lib/elapsed"
import { cn } from "@/lib/utils"
import type { Phase } from "@/lib/useBoot"

/** BootScreen 接收的阶段:Phase 去掉 error(错误分发在 App 完成,此处只多不少)。 */
export type BootPhase = Exclude<Phase, "error">

export function BootScreen({
  phase,
  progress,
  stage,
  nodeVersion,
  elapsedSecs,
  exiting = false,
  onExitAnimationEnd,
}: {
  phase: BootPhase
  /** 安装模拟进度 0-100(null = 非安装阶段) */
  progress: number | null
  /** 安装子阶段键后缀("fetching"|"reifying"|"finishing") */
  stage: string | null
  /** Node 检测结果,仅 checking 阶段有值(null = 检测中/非 checking 阶段) */
  nodeVersion: string | null
  /** 从 boot 启动起的累计秒数(null = 快照未到,尚无起点) */
  elapsedSecs: number | null
  /** 就绪退出动画中(#4):整屏溶解 + 圆环收缩收敛 */
  exiting?: boolean
  /** 退出动画结束(CSS onAnimationEnd,动画名过滤后上报) */
  onExitAnimationEnd?: () => void
}) {
  const { t } = useTranslation()
  // hint 键仅 installing/starting 存在;defaultValue 兜底让其余阶段静默无提示
  const hint = t(`boot.${phase}.hint`, { defaultValue: "" })
  // 耗时文案:分钟级用「N 分 M 秒」,秒级用「N 秒」(模板键分列,插值不拼串)
  let elapsedText: string | null = null
  if (elapsedSecs !== null) {
    const { minutes, seconds } = formatElapsed(elapsedSecs)
    elapsedText =
      minutes > 0
        ? t("boot.elapsed.minSec", { m: minutes, s: seconds })
        : t("boot.elapsed.sec", { s: seconds })
  }
  const installing = phase === "installing" && progress !== null
  return (
    <main
      className={cn(
        "flex h-full w-full flex-col items-center justify-center bg-background text-foreground",
        exiting && "boot-exit",
      )}
      onAnimationEnd={
        exiting
          ? (e) => {
              // 只认退出动画的结束事件(子元素动画冒泡也会触达,过滤防误信号)
              if (e.animationName === "boot-exit" || e.animationName === "boot-exit-ring") {
                onExitAnimationEnd?.()
              }
            }
          : undefined
      }
    >
      {/* 仪表行:圆环仪表 + 右侧读数列;进度轨与读数列同宽对齐 */}
      <div className="flex w-full max-w-lg items-center gap-8 px-6">
        {/* 仪表环:静止刻度环 + 自转活动弧;就绪退出时外层收缩(缩到中心点消失) */}
        <div className={cn("relative size-[124px] shrink-0", exiting && "boot-exit-ring")} aria-hidden>
          <div className="absolute inset-0 rounded-full border-[3px] border-primary/10" />
          <div className="absolute inset-0 animate-spin rounded-full border-[3px] border-transparent border-t-primary" />
        </div>

        <div className="flex min-w-0 flex-1 flex-col gap-6" aria-live="polite">
          <div>
            <h1 className="text-lg font-medium leading-snug">{t(`boot.${phase}.title`)}</h1>
            {hint && <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{hint}</p>}
            {/* #13:checking 阶段检测结果可视化(nodeVersion 仅此时有值) */}
            {phase === "checking" && nodeVersion && (
              <p className="mt-1.5 flex items-center gap-1.5 text-sm text-emerald-600 dark:text-emerald-500">
                <Check className="size-4 shrink-0" aria-hidden />
                {t("boot.checking.nodeFound", { version: nodeVersion })}
              </p>
            )}
          </div>

          <div className="flex flex-col gap-1.5">
            {installing ? (
              /* 安装中:确定进度(阶段文案 + 百分比),boot 与升级链共用组件 */
              <InstallProgress progress={progress} stage={stage} i18nPrefix="boot" />
            ) : (
              /* 不确定进度条(滑动指示,不显示假百分比) */
              <ProgressRail value={null} />
            )}
            {/* 耗时读数:右对齐仪表读数;aria-live="off" 每秒更新不打扰读屏 */}
            {elapsedText && (
              <p className="text-right text-xs tabular-nums text-muted-foreground" aria-live="off">
                {elapsedText}
              </p>
            )}
          </div>
        </div>
      </div>
    </main>
  )
}
