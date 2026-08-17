// Root app.壳页常驻(ADR 0001 / #36):单一 tauri WebView 窗口内是常驻壳页——
// 菜单条占位区 + dsh iframe 容器 + 全屏覆盖层挂载点。
// M5(#40):覆盖层互斥由 deriveOverlay 纯函数编排(features/shell/derive.ts,
// 优先级 Error > Upgrade > Boot;dsh 意外退出经 reaper 的 dsh-exited 事件 →
// dshDown,升级流水线在途抑制误报)。boot / dsh 升级 / 意外退出三态 + 应用
// 升级卡(决策面,不参与 derive 链,优先级最低)渲染在 ShellLayout 的浮层
// 挂载点;菜单条不参与互斥,任何阶段常驻(ShellLayout 结构保证)。
// dsh URL 单一事实来源在 Rust(record_dsh_url → dsh-url 事件/快照,useBoot
// 持有),壳页只负责 set iframe.src;dsh 就绪(URL 到达)后播放退出过渡动画
// 溶解 boot 覆盖层,揭示 iframe(useBootExit,fallback 兜底,动画不阻塞呈现)。
// #39:shell-dialog 弹窗(AlertDialog/toast)与更新进度浮层(UpdateFloat)是
// 非互斥层,不参与 overlay 链——浮层盖在 iframe 上但不阻止交互。
import { BootScreen } from "@/components/boot/BootScreen"
import { ErrorScreen } from "@/components/boot/ErrorScreen"
import { CrashScreen } from "@/components/shell/CrashScreen"
import { ShellDialogs } from "@/components/shell/ShellDialogs"
import { ShellLayout } from "@/components/shell/ShellLayout"
import { UpdateCard } from "@/components/update/UpdateCard"
import { UpdateFloat } from "@/components/update/UpdateFloat"
import { UpgradeScreen } from "@/components/upgrade/UpgradeScreen"
import { Toaster } from "@/components/ui/sonner"
import { deriveOverlay } from "@/features/shell/derive"
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
    dshDown,
    dshExitCode,
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

  // 覆盖层互斥(纯函数,derive.test.ts 守住三路状态组合):Error > Upgrade >
  // Boot;bootRevealed(done)让就绪后的退出动画撑住 boot 覆盖层直到揭示
  const overlayKind = deriveOverlay({
    bootPhase: phase,
    bootRevealed: done,
    upgradeStatus: dshUpgrade.status,
    upgradeVisible: dshUpgrade.visible,
    dshDown,
  })

  // 单一表达式(无浮层时传 null):ShellLayout 靠 children 真假切换浮层挂载点
  // 的 pointer-events——多个条件兄弟表达式会形成 [null,…] 数组,空浮层挡
  // 不住 iframe 点击,故这里只产出一个值。
  // 顺序:derive 链三态(错误/升级/boot)优先;应用升级卡(决策面,不参与
  // derive 链)兜底——升级卡只承载用户决策,不打断更紧急的覆盖层。
  const overlay = overlayKind ? (
    overlayKind.kind === "error" ? (
      // dsh 意外退出:reaper → dsh-exited → 全屏错误覆盖层 + [重试](重跑 boot)
      <CrashScreen exitCode={dshExitCode} retry={retry} quit={quit} />
    ) : overlayKind.kind === "upgrade" ? (
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
    ) : overlayKind.phase === "error" ? (
      // boot 失败页(含 Node 引导页分支,ErrorScreen 内部判别)
      <ErrorScreen error={error} logs={logs} retry={retry} quit={quit} />
    ) : (
      <BootScreen
        // overlayKind.phase 已由上方判别收窄到非 error 阶段(与 phase 同值)
        phase={overlayKind.phase}
        progress={progress}
        stage={stage}
        nodeVersion={nodeVersion}
        elapsedSecs={elapsedSecs}
        exiting={exiting}
        onExitAnimationEnd={onExitAnimationEnd}
      />
    )
  ) : update.visible ? (
    <UpdateCard
      status={update.status}
      version={update.version}
      currentVersion={update.currentVersion}
      notes={update.notes}
      error={update.error}
      onApply={update.applyUpdate}
      onDismiss={update.dismiss}
      onOpenReleases={update.openReleases}
    />
  ) : null

  // 更新进度浮层(右下角非模态):downloading/ready 不参与互斥 overlay 链,
  // 独立渲染盖在 iframe 之上(fixed 定位,不阻止 dsh 交互,#31 拍板)
  const updateFloat =
    update.status === "downloading" || update.status === "ready" ? (
      <UpdateFloat
        status={update.status}
        downloadedBytes={update.downloadedBytes}
        totalBytes={update.totalBytes}
        onRestart={update.restartNow}
        onDismiss={update.dismiss}
      />
    ) : null

  return (
    <>
      <ShellLayout dshUrl={dshUrl}>{overlay}</ShellLayout>
      {updateFloat}
      {/* 壳页弹窗(AlertDialog/toast):Rust shell-dialog 事件驱动 */}
      <ShellDialogs />
      {/* Sonner toast 容器:右下角(与更新浮层同角,疑点 8 结论:接受叠放) */}
      <Toaster position="bottom-right" />
    </>
  )
}
