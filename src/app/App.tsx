// Root app.启动编排:按 boot 阶段分发渲染;升级卡片优先于 boot 分发——
// 挂载时若升级状态活跃且快照已是 ready(本地页由升级流程导航回来,dsh 已在跑),
// 渲染对应升级卡片;否则走 boot 分发(现状逻辑不变;全新启动时 ready 事件到达后
// Rust 即将导航去 dsh 页,靠 useBoot 的 mountSnapshotReady 区分,不中途切卡片)。
// ready 由 Rust 侧接管(窗口 navigate 到 dsh Web UI),前端只短暂显示过渡。
// 升级卡优先级(#17):dsh 升级卡(upgrade.*) → 应用升级卡(update.*) → boot 分发。
import { BootScreen } from "@/components/boot/BootScreen"
import { ErrorScreen } from "@/components/boot/ErrorScreen"
import { UpgradeCard } from "@/components/update/UpgradeCard"
import { UpgradeScreen } from "@/components/upgrade/UpgradeScreen"
import { useBoot } from "@/lib/useBoot"
import { useDshUpgrade, isActiveDshUpgradeStatus } from "@/lib/useDshUpgrade"
import { useThemeSync } from "@/lib/useThemeSync"
import { isActiveUpdateStatus, useUpdateCheck } from "@/lib/useUpdateCheck"

export default function App() {
  const {
    phase,
    error,
    logs,
    progress,
    stage,
    nodeVersion,
    elapsedSecs,
    retry,
    quit,
    mountSnapshotReady,
  } = useBoot()
  // 主题同步:Rust 下发生效主题 → <html>.dark(boot UI 全程生效)
  useThemeSync()
  // 升级状态镜像(Rust 侧单一事实源,见 useDshUpgrade / useUpdateCheck)
  const dshUpgrade = useDshUpgrade()
  const update = useUpdateCheck()

  // #3 §5:挂载时先查升级状态,有活跃态(且快照已 ready) → 升级卡片;
  // 否则走 boot 分发。boot 命令在升级页挂载时仍被调用(phase=Ready 无副作用)。
  if (mountSnapshotReady && isActiveDshUpgradeStatus(dshUpgrade.status)) {
    return (
      <UpgradeScreen
        status={dshUpgrade.status}
        version={dshUpgrade.version}
        currentVersion={dshUpgrade.currentVersion}
        phase={dshUpgrade.phase}
        progress={dshUpgrade.progress}
        stage={dshUpgrade.stage}
        error={dshUpgrade.error}
        onConfirm={dshUpgrade.confirm}
        onDismiss={dshUpgrade.dismiss}
      />
    )
  }

  if (mountSnapshotReady && isActiveUpdateStatus(update.status)) {
    return (
      <UpgradeCard
        status={update.status}
        version={update.version}
        currentVersion={update.currentVersion}
        notes={update.notes}
        error={update.error}
        downloadedBytes={update.downloadedBytes}
        totalBytes={update.totalBytes}
        onApply={update.applyUpdate}
        onRestart={update.restartNow}
        onDismiss={update.dismiss}
        onOpenReleases={update.openReleases}
      />
    )
  }

  if (phase === "error") {
    // error 是结构化失败原因,ErrorScreen 渲染时翻译(兜底 errors.unknown 在彼处)
    return <ErrorScreen error={error} logs={logs} retry={retry} quit={quit} />
  }
  // idle(快照未到)/checking/installing/starting/ready 均由 BootScreen 呈现对应文案
  return (
    <BootScreen
      phase={phase}
      progress={progress}
      stage={stage}
      nodeVersion={nodeVersion}
      elapsedSecs={elapsedSecs}
    />
  )
}
