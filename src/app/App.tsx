// Root app.启动编排:按 boot 阶段分发渲染。
// ready 由 Rust 侧接管(窗口 navigate 到 dsh Web UI),前端只短暂显示过渡。
import { BootScreen } from "@/components/boot/BootScreen"
import { ErrorScreen } from "@/components/boot/ErrorScreen"
import { useBoot } from "@/lib/useBoot"
import { useThemeSync } from "@/lib/useThemeSync"

export default function App() {
  const { phase, error, logs, retry, quit } = useBoot()
  // 主题同步:Rust 下发生效主题 → <html>.dark(boot UI 全程生效)
  useThemeSync()

  if (phase === "error") {
    // error 是结构化失败原因,ErrorScreen 渲染时翻译(兜底 errors.unknown 在彼处)
    return <ErrorScreen error={error} logs={logs} retry={retry} quit={quit} />
  }
  // idle(快照未到)/checking/installing/starting/ready 均由 BootScreen 呈现对应文案
  return <BootScreen phase={phase} />
}
