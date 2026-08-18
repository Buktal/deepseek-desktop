// 启动页(idle / checking / installing / starting / ready 共用):「启动仪表」布局。
// 设计(#20 审核定稿 + 品牌化重塑):仪表盘在左、阶段读数在右的横排仪表行——
// 外壳是启动仪表而非品牌页(无 Logo,品牌由托盘承担),横排让 920px 宽的窗口
// 不被竖排堆叠浪费。
// 仪表盘 = 60 道刻度表圈(每 5 道一道主刻度)+ 自转扫描弧 + 盘心计时读数。
// 扫描弧用鲸标同源靛蓝渐变(index.css --brand-grad-*):品牌色在 UI 的唯一
// 落点,不铺到按钮/大面;耗时从进度轨旁小字迁入盘心,chronometer「分:秒」
// 读数(formatClock,语言中立),每秒更新 aria-live="off" 不打扰读屏。
// 就绪退出时外层收缩收敛(#4,缩放在外层 wrapper,自转在内层元素,无 transform 冲突)。
// 进度轨对齐在右侧读数列下:安装中显示确定进度(InstallProgress,阶段 + 百分比),
// 其余阶段不确定进度(滑动指示,不显示假百分比)。
// #13:checking 阶段携带 nodeVersion 时显示检测结果(「检测到 Node.js vX」)。
// M5(#40):本组件是全屏覆盖层的 boot 状态面(deriveOverlay 编排),渲染在
// ShellLayout 浮层挂载点,菜单条常驻。
import { Check } from "lucide-react"
import type { CSSProperties } from "react"
import { useTranslation } from "react-i18next"

import { InstallProgress } from "@/components/install/InstallProgress"
import {
  BOOT_EXIT_ANIMATION_MS,
  BOOT_EXIT_ANIMATION_NAME,
  BOOT_EXIT_RING_ANIMATION_NAME,
} from "@/lib/bootExit"
import { formatClock } from "@/lib/elapsed"
import type { InstallStage } from "@/lib/installStage"
import { cn } from "@/lib/utils"
import type { Phase } from "@/lib/useBoot"

/** BootScreen 接收的阶段:Phase 去掉 error(错误分发在 App 完成,此处只多不少)。 */
export type BootPhase = Exclude<Phase, "error">

/** 表圈刻度:60 道径向刻度(每 5 道一道主刻度),模块级预计算极坐标→线段坐标。 */
const TICKS = Array.from({ length: 60 }, (_, i) => {
  const major = i % 5 === 0
  const rad = ((i * 6 - 90) * Math.PI) / 180
  const cos = Math.cos(rad)
  const sin = Math.sin(rad)
  const inner = major ? 40 : 42
  return {
    i,
    major,
    x1: 50 + inner * cos,
    y1: 50 + inner * sin,
    x2: 50 + 44 * cos,
    y2: 50 + 44 * sin,
  }
})

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
  /** 安装子阶段键后缀(InstallStage 联合,见 src/lib/installStage.ts) */
  stage: InstallStage | null
  /** Node 检测结果,仅 checking 阶段有值(null = 检测中/非 checking 阶段) */
  nodeVersion: string | null
  /** 从 boot 启动起的累计秒数(null = 快照未到,尚无起点;盘心按 0:00 起) */
  elapsedSecs: number | null
  /** 就绪退出动画中(#4):整屏溶解 + 圆环收缩收敛 */
  exiting?: boolean
  /** 退出动画结束(CSS onAnimationEnd,动画名过滤后上报) */
  onExitAnimationEnd?: () => void
}) {
  const { t } = useTranslation()
  // hint 键仅 installing/starting 存在;defaultValue 兜底让其余阶段静默无提示
  const hint = t(`boot.${phase}.hint`, { defaultValue: "" })
  return (
    <main
      className={cn(
        "flex h-full w-full flex-col items-center justify-center bg-background text-foreground",
        exiting && "boot-exit",
      )}
      // 退出动画时长契约(Q1):bootExit.ts 单一出口,经 CSS 变量消费
      // (index.css 的 boot-exit / boot-exit-ring 动画时长不再硬编码);
      // CSSProperties 的封闭类型不支持自定义属性,需断言
      style={{ "--boot-exit-ms": `${BOOT_EXIT_ANIMATION_MS}ms` } as CSSProperties}
      onAnimationEnd={
        exiting
          ? (e) => {
              // 只认退出动画的结束事件(子元素动画冒泡也会触达,过滤防误信号)
              if (
                e.animationName === BOOT_EXIT_ANIMATION_NAME ||
                e.animationName === BOOT_EXIT_RING_ANIMATION_NAME
              ) {
                onExitAnimationEnd?.()
              }
            }
          : undefined
      }
    >
      {/* 仪表行:仪表盘 + 右侧读数列;进度轨与读数列同宽对齐 */}
      <div className="flex w-full max-w-lg items-center gap-8 px-6">
        {/* 仪表盘:刻度表圈 + 品牌渐变扫描弧(自转)+ 盘心计时;
            就绪退出时外层收缩(缩到中心点消失) */}
        <div className={cn("relative size-[124px] shrink-0", exiting && "boot-exit-ring")}>
          {/* 表圈:静止刻度(主刻度每 5 道一道,机械仪表 bezel 形态) */}
          <svg viewBox="0 0 100 100" aria-hidden className="absolute inset-0 text-primary">
            {TICKS.map((tick) => (
              <line
                key={tick.i}
                x1={tick.x1}
                y1={tick.y1}
                x2={tick.x2}
                y2={tick.y2}
                stroke="currentColor"
                strokeWidth={tick.major ? 1.8 : 1}
                strokeLinecap="round"
                opacity={tick.major ? 0.4 : 0.15}
              />
            ))}
          </svg>
          {/* 扫描弧:鲸标同源靛蓝渐变,自转(reduced-motion 下停转,盘心读数仍在) */}
          <svg viewBox="0 0 100 100" aria-hidden className="absolute inset-0 animate-spin">
            <defs>
              <linearGradient id="boot-sweep" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0" style={{ stopColor: "var(--brand-grad-from)" }} />
                <stop offset="1" style={{ stopColor: "var(--brand-grad-to)" }} />
              </linearGradient>
            </defs>
            <circle
              cx="50"
              cy="50"
              r="46.5"
              fill="none"
              stroke="url(#boot-sweep)"
              strokeWidth="3.2"
              strokeLinecap="round"
              strokeDasharray="77 215"
            />
          </svg>
          {/* 盘心计时读数:chronometer 分:秒;每秒更新,aria-live off 不打扰读屏 */}
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-1" aria-live="off">
            <span className="text-[10px] text-muted-foreground">{t("boot.elapsed.label")}</span>
            <span className="text-xl leading-none font-medium tabular-nums">
              {formatClock(elapsedSecs ?? 0)}
            </span>
          </div>
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

          {/* 进度:安装中确定进度(阶段文案 + 百分比),其余阶段不确定进度
              (滑动指示,不显示假百分比)——boot 与升级链共用 InstallProgress */}
          <InstallProgress progress={progress} stage={stage} i18nPrefix="boot" />
        </div>
      </div>
    </main>
  )
}
