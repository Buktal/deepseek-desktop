// 安装进度展示:阶段文案 + 百分比 + 确定进度条(#7)。
// boot 安装与 dsh 升级链共用同一组件(i18nPrefix 区分文案键命名空间,
// 升级落地时传 "upgrade",键位 upgrade.installing.* 见 #3 §6)。
// 数据来自 Rust 侧 boot-state 事件的 progress/stage 字段:模拟推进 + npm 进程
// 退出校准 100%(Rust 侧语义,本组件只做纯展示,不做任何业务判断)。
// 文案键:${i18nPrefix}.installing.stage.<stage> / ${i18nPrefix}.installing.progress,
// 动态数值一律插值(#12 约束)。

import { useTranslation } from "react-i18next"

import { Progress } from "@/components/ui/progress"

export function InstallProgress({
  progress,
  stage,
  i18nPrefix,
}: {
  /** 确定进度 0-100(Rust 侧模拟 + 校准值) */
  progress: number
  /** 子阶段键后缀("fetching"|"reifying"|"finishing"),缺省时只显示百分比 */
  stage?: string | null
  /** 文案键命名空间:boot 安装用 "boot",dsh 升级链用 "upgrade" */
  i18nPrefix: "boot" | "upgrade"
}) {
  const { t } = useTranslation()
  const stageText = stage
    ? t(`${i18nPrefix}.installing.stage.${stage}`, { defaultValue: "" })
    : ""
  return (
    <div className="flex w-72 flex-col gap-2">
      <div className="flex items-baseline justify-between text-sm">
        <span className="text-muted-foreground">{stageText}</span>
        {/* 百分比符号是语言中立符号,随 #12 惯例进模板键(见 update.downloaded) */}
        <span className="tabular-nums text-muted-foreground">
          {t(`${i18nPrefix}.installing.progress`, { pct: progress })}
        </span>
      </div>
      {/* shadcn Progress(value 驱动宽度 + aria-valuenow;Base UI 自动带进度语义) */}
      <Progress
        value={progress}
        aria-label={stageText || undefined}
        className="h-1.5 w-full [&>[data-slot='progress-track']]:h-full [&>[data-slot='progress-track']]:rounded-full"
      />
    </div>
  )
}
