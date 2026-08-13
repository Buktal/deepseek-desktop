// Root app.启动编排:按 boot 阶段分发渲染。
// ready 由 Rust 侧接管(窗口 navigate 到 dsh Web UI),前端不做任何事。
import { BootScreen } from "@/components/boot/BootScreen"
import { ErrorScreen } from "@/components/boot/ErrorScreen"
import { useBoot } from "@/lib/useBoot"

export default function App() {
  const { phase, error, logs, retry, quit } = useBoot()

  if (phase === "error") {
    return <ErrorScreen message={error ?? "未知错误"} logs={logs} retry={retry} quit={quit} />
  }
  if (phase === "checking" || phase === "installing" || phase === "starting") {
    return <BootScreen phase={phase} />
  }
  // idle(快照未到)/ready(即将导航):短暂显示 loading 过渡
  return <BootScreen phase="checking" />
}
