// loading 页:idle / checking / installing / starting / ready 共用
// 只显示阶段 + 不确定进度条;日志不推流(异常时经错误页携带)
// 外壳界面不展示 Logo(品牌由托盘图标承担),旋转圆环仅作 loading 指示
import { useTranslation } from "react-i18next"

export type BootPhase = "idle" | "checking" | "installing" | "starting" | "ready"

export function BootScreen({ phase }: { phase: BootPhase }) {
  const { t } = useTranslation()
  // hint 键仅 installing/starting 存在;defaultValue 兜底让其余阶段静默无提示
  const hint = t(`boot.${phase}.hint`, { defaultValue: "" })
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
      </div>

      {/* 不确定进度条(滑动指示,不显示假百分比) */}
      <div className="h-1.5 w-72 overflow-hidden rounded-full bg-muted" role="progressbar">
        <div className="h-full w-1/3 animate-loading-slide rounded-full bg-indigo-500" />
      </div>
    </main>
  )
}
