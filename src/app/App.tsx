// Root app.壳页常驻(ADR 0001 / #36):单一 tauri WebView 窗口内是常驻壳页——
// 菜单条占位区 + dsh iframe 容器 + 浮层挂载点;boot / 升级卡 / 更新卡 / 错误页
// 全部作为浮层盖在 iframe 之上(浮层编排 M5 收口,本票 M1 只做骨架)。
// 浮层互斥与优先级:升级卡 → 更新卡 → 错误页 → boot(升级卡优先,与原分发一致)。
// dsh URL 单一事实来源在 Rust(record_dsh_url → dsh-url 事件/快照,useBoot 持有),
// 壳页只负责 set iframe.src;dsh 就绪(URL 到达)后播放退出过渡动画溶解 boot
// 浮层,揭示 iframe(useBootExit,fallback 兜底,动画不阻塞 dsh 呈现)。
// 卡片可见性:升级/更新卡由状态 + 托盘显式请求驱动(isUpgradeCardVisible /
// isUpdateCardVisible),不再依赖「页面挂载」信号(壳页不再重新挂载)。
import { BootScreen } from "@/components/boot/BootScreen"
import { ErrorScreen } from "@/components/boot/ErrorScreen"
import { ShellLayout } from "@/components/shell/ShellLayout"
import { UpdateCard } from "@/components/update/UpdateCard"
import { UpgradeScreen } from "@/components/upgrade/UpgradeScreen"
import { useBoot } from "@/lib/useBoot"
import { useBootExit } from "@/lib/useBootExit"
import { useDshUpgrade } from "@/lib/useDshUpgrade"
import { useExternalLinks } from "@/lib/useExternalLinks"
import { useThemeSync } from "@/lib/useThemeSync"
import { useUpdateCheck } from "@/lib/useUpdateCheck"

export default function App() {
  const {
    phase,
    error,
    logs,
    progress,
    stage,
    nodeVersion,
    elapsedSecs,
    dshUrl,
    retry,
    quit,
  } = useBoot()
  // 主题同步:Rust 下发生效主题 → <html>.dark(壳页全程生效)
  useThemeSync()
  // 页面层外链拦截接收端:dsh iframe 命中外链 → postMessage → opener 开浏览器
  useExternalLinks()
  // 升级状态镜像(Rust 侧单一事实源,见 useDshUpgrade / useUpdateCheck)
  const dshUpgrade = useDshUpgrade()
  const update = useUpdateCheck()

  // boot 浮层退出过渡:ready 且 URL 已推给壳页(iframe 开始加载)即播放溶解
  // 动画揭示 dsh;动画纯装饰,fallback 定时器兜底,不阻塞呈现
  const { exiting, done, onExitAnimationEnd } = useBootExit({
    ready: phase === "ready" && dshUrl !== null,
  })

  // 浮层可见性:升级卡优先(与 #3 §5 分发一致),其次更新卡,再次错误页,
  // boot 兜底(idle 也显示 boot 页,避免挂载瞬间的空壳闪烁)。
  const showBootOverlay =
    phase === "idle" ||
    phase === "error" ||
    phase === "checking" ||
    phase === "installing" ||
    phase === "starting" ||
    (phase === "ready" && !done)
  // 单一表达式(无浮层时传 null):ShellLayout 靠 children 真假切换浮层挂载点
  // 的 pointer-events——多个条件兄弟表达式会形成 [null,…] 数组,空浮层挡
  // 不住 iframe 点击,故这里只产出一个值。
  const overlay =
    dshUpgrade.visible ? (
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
    ) : update.visible ? (
      <UpdateCard
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
    ) : showBootOverlay ? (
      phase === "error" ? (
        <ErrorScreen error={error} logs={logs} retry={retry} quit={quit} />
      ) : (
        <BootScreen
          phase={phase}
          progress={progress}
          stage={stage}
          nodeVersion={nodeVersion}
          elapsedSecs={elapsedSecs}
          exiting={exiting}
          onExitAnimationEnd={onExitAnimationEnd}
        />
      )
    ) : null

  return <ShellLayout dshUrl={dshUrl}>{overlay}</ShellLayout>
}
