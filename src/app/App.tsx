// Root app.启动编排:按 boot 阶段分发渲染。
// ready 由 Rust 侧接管(窗口 navigate 到 dsh Web UI),前端只短暂显示过渡。
import { BootScreen } from "@/components/boot/BootScreen"
import { ErrorScreen } from "@/components/boot/ErrorScreen"
import { useBoot } from "@/lib/useBoot"

export default function App() {
  const { phase, error, logs, retry, quit } = useBoot()

  if (phase === "error") {
    return <ErrorScreen message={error ?? "未知错误"} logs={logs} retry={retry} quit={quit} />
  }
  // idle(快照未到)/checking/installing/starting/ready 均由 BootScreen 呈现对应文案
  return <BootScreen phase={phase} />
}
