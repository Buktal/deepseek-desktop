// 进度展示:阶段文案 + 百分比 + 进度条(#7),boot 安装与 dsh 升级链共用
// (i18nPrefix 区分文案键命名空间)。
// progress = 0-100 显示确定进度(shadcn Progress,阶段文案 + 百分比);
// null = 不确定进度(滑动指示,不显示假百分比)——boot 非安装阶段与升级链
// killing/verifying/starting 复用同一条滑动指示(M5 迁移后 ProgressRail 退役,
// 两种形态归并到本组件:宽度/圆角/配色单一事实来源,不复制实现)。
// 数据来自 Rust 侧 boot-state / upgrade-state 事件的 progress/stage 字段:
// 模拟推进 + npm 进程退出校准 100%(Rust 侧语义,本组件只做纯展示)。
// 文案键:${i18nPrefix}.installing.stage.<stage> / ${i18nPrefix}.installing.progress,
// 动态数值一律插值(#12 约束)。

import { useTranslation } from "react-i18next"

import { Progress } from "@/components/ui/progress"

export function InstallProgress({
  progress,
  stage,
  i18nPrefix,
}: {
  /** 确定进度 0-100(Rust 侧模拟 + 校准值);null = 不确定进度(滑动指示) */
  progress: number | null
  /** 子阶段键后缀("fetching"|"reifying"|"finishing"),缺省时只显示百分比 */
  stage?: string | null
  /** 文案键命名空间:boot 安装用 "boot",dsh 升级链用 "upgrade" */
  i18nPrefix: "boot" | "upgrade"
}) {
  const { t } = useTranslation()
  if (progress !== null) {
    const stageText = stage
      ? t(`${i18nPrefix}.installing.stage.${stage}`, { defaultValue: "" })
      : ""
    return (
      <div className="flex w-full flex-col gap-2">
        <div className="flex items-baseline justify-between text-sm">
          <span className="text-muted-foreground">{stageText}</span>
          {/* 百分比符号是语言中立符号,随 #12 惯例进模板键(见 update.downloaded) */}
          <span className="tabular-nums text-muted-foreground">
            {t(`${i18nPrefix}.installing.progress`, { pct: progress })}
          </span>
        </div>
        <Progress
          value={progress}
          aria-label={stageText || undefined}
          className="h-1.5 w-full [&>[data-slot='progress-track']]:h-full [&>[data-slot='progress-track']]:rounded-full"
        />
      </div>
    )
  }
  // 不确定进度:滑动指示(不显示假百分比;纯装饰,阶段标题承担语义)
  return (
    <div
      role="progressbar"
      className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
    >
      <div className="h-full w-1/3 animate-loading-slide rounded-full bg-primary" />
    </div>
  )
}
