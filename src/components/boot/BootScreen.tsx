// loading 页:checking / installing / starting 三态共用
// 只显示阶段 + 不确定进度条;日志不推流(异常时经错误页携带)
import { Logo } from "@/components/Logo"

const COPY: Record<"checking" | "installing" | "starting", { title: string; hint?: string }> = {
  checking: { title: "正在检查运行环境…" },
  installing: { title: "正在安装 dsh…", hint: "首次安装可能需要几分钟,请耐心等待" },
  starting: { title: "正在启动 dsh…", hint: "首次启动需要初始化环境,可能需要一两分钟" },
}

export function BootScreen({ phase }: { phase: "checking" | "installing" | "starting" }) {
  const copy = COPY[phase]
  return (
    <main className="flex h-screen w-screen flex-col items-center justify-center gap-7 bg-background text-foreground">
      {/* Logo + 旋转圆环 */}
      <div className="relative flex items-center justify-center">
        <div
          aria-hidden
          className="absolute size-[124px] animate-spin rounded-full border-[3px] border-transparent border-t-indigo-500/80"
        />
        <Logo size={92} />
      </div>

      <div className="text-center">
        <h1 className="text-lg font-medium">{copy.title}</h1>
        {copy.hint && <p className="mt-1 text-sm text-muted-foreground">{copy.hint}</p>}
      </div>

      {/* 不确定进度条(滑动指示,不显示假百分比) */}
      <div className="h-1.5 w-72 overflow-hidden rounded-full bg-muted" role="progressbar">
        <div className="h-full w-1/3 animate-loading-slide rounded-full bg-indigo-500" />
      </div>
    </main>
  )
}
