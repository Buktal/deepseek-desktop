// loading 页:idle / checking / installing / starting / ready 共用
// 只显示阶段 + 进度;日志不推流(异常时经错误页携带)
// 外壳界面不展示 Logo(品牌由托盘图标承担),旋转圆环仅作 loading 指示
// #7:installing 显示确定进度(模拟 + npm 退出校准,InstallProgress 组件);
// 其余阶段不确定进度条(滑动指示,不显示假百分比);全程显示耗时
// (boot.elapsed.* 键,秒级累计,checking/installing/starting 持续可见)
// #13:checking 阶段携带 nodeVersion 时显示检测结果(「检测到 Node.js vX」),
// 让用户看到检测在推进(checking 还要做 npm root -g 等检查,有展示窗口)
import { Check } from "lucide-react"
import { useTranslation } from "react-i18next"

import { InstallProgress } from "@/components/install/InstallProgress"
import { formatElapsed } from "@/lib/elapsed"

export type BootPhase = "idle" | "checking" | "installing" | "starting" | "ready"

export function BootScreen({
  phase,
  progress,
  stage,
  nodeVersion,
  elapsedSecs,
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
    <main className="flex h-screen w-screen flex-col items-center justify-center gap-7 bg-background text-foreground">
      {/* 旋转圆环:loading 指示 */}
      <div
        aria-hidden
        className="size-[124px] animate-spin rounded-full border-[3px] border-transparent border-t-indigo-500/80"
      />

      <div className="text-center">
        <h1 className="text-lg font-medium">{t(`boot.${phase}.title`)}</h1>
        {hint && <p className="mt-1 text-sm text-muted-foreground">{hint}</p>}
        {/* #13:checking 阶段检测结果可视化(nodeVersion 仅此时有值) */}
        {phase === "checking" && nodeVersion && (
          <p className="mt-1 flex items-center justify-center gap-1 text-sm text-emerald-600">
            <Check className="size-4" aria-hidden />
            {t("boot.checking.nodeFound", { version: nodeVersion })}
          </p>
        )}
      </div>

      {installing ? (
        /* 安装中:确定进度(阶段文案 + 百分比),boot 与升级链共用组件 */
        <InstallProgress progress={progress} stage={stage} i18nPrefix="boot" />
      ) : (
        /* 不确定进度条(滑动指示,不显示假百分比) */
        <div className="h-1.5 w-72 overflow-hidden rounded-full bg-muted" role="progressbar">
          <div className="h-full w-1/3 animate-loading-slide rounded-full bg-indigo-500" />
        </div>
      )}

      {/* boot 全程耗时:checking/installing/starting 持续显示 */}
      {elapsedText && (
        <p className="text-xs text-muted-foreground" aria-live="off">
          {elapsedText}
        </p>
      )}
    </main>
  )
}
