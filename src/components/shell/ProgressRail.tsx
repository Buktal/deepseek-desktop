// 进度条统一组件:确定进度(shadcn Progress)与不确定进度(滑动指示)共用一条实现。
// 此前 BootScreen / UpgradeScreen / UpgradeCard 各自复制了一份「滑动条」结构,
// 且三处硬编码 accent 色(indigo-500)与 InstallProgress 的 primary token 分叉——
// 归并到本组件后,宽度/圆角/配色只有一个事实来源(primary token)。
// value = 0-100 显示确定进度;null = 不确定(滑动指示,不显示假百分比)。

import { Progress } from "@/components/ui/progress"
import { cn } from "@/lib/utils"

export function ProgressRail({
  value,
  className,
  "aria-label": ariaLabel,
}: {
  /** 确定进度 0-100;null = 不确定进度(滑动指示) */
  value: number | null
  className?: string
  "aria-label"?: string
}) {
  if (value !== null) {
    return (
      <Progress
        value={value}
        aria-label={ariaLabel}
        className={cn(
          "h-1.5 w-full [&>[data-slot='progress-track']]:h-full [&>[data-slot='progress-track']]:rounded-full",
          className,
        )}
      />
    )
  }
  return (
    <div
      role="progressbar"
      aria-label={ariaLabel}
      className={cn("h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}
    >
      <div className="h-full w-1/3 animate-loading-slide rounded-full bg-primary" />
    </div>
  )
}
