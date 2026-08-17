// ⚠️ PROTOTYPE(#30)——throwaway,分支 prototype/shell-ui,不合并 main。
// 问题:壳页常驻架构下,标题栏(菜单下拉 + 三平台窗口控制布局)、关闭三选弹窗、
// 更新提示浮层盖在 dsh 之上,长什么样?对着实物拍审美与交互。
// 怎么跑:yarn web:dev 后开
//   http://localhost:1420/?prototype=shell
//   可带 &platform=macos|windows|linux(&scene=… &menu=open),URL 即分享。
// 决策依据:#28(三平台窗口控制分治)、#31(六弹窗形态)、#32(全屏覆盖层互斥)、
// #33(MenuSnapshot 纯镜像渲染)。mock 全内联本文件,不接真 Rust。
// 视觉沿用 src/index.css 的 base-maia 中性色,暗色为默认语境(主题菜单可切)。

import { useEffect, useMemo, useState } from "react"
import { toast } from "sonner"
import {
  CircleAlert,
  CircleArrowUp,
  Globe,
  Loader2,
  Menu as MenuIcon,
  Minus,
  PartyPopper,
  RotateCw,
  Square,
  X,
} from "lucide-react"

import { ProgressRail } from "@/components/shell/ProgressRail"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Toaster } from "@/components/ui/sonner"
import { cn } from "@/lib/utils"

// ---------- URL 参数模型(可分享、可截图) ----------

type Platform = "macos" | "windows" | "linux"
type SceneId =
  | "normal"
  | "update-found"
  | "update-downloading"
  | "update-ready"
  | "close-ask"
  | "boot"
  | "dsh-upgrade"
  | "dsh-crashed"
  | "toast-latest"
  | "toast-error"

const PLATFORMS: { id: Platform; label: string }[] = [
  { id: "macos", label: "macOS" },
  { id: "windows", label: "Windows" },
  { id: "linux", label: "Linux" },
]

const SCENES: { id: SceneId; label: string }[] = [
  { id: "normal", label: "正常" },
  { id: "update-found", label: "发现新版" },
  { id: "update-downloading", label: "下载中" },
  { id: "update-ready", label: "下载完成" },
  { id: "close-ask", label: "关闭三选" },
  { id: "boot", label: "boot" },
  { id: "dsh-upgrade", label: "dsh 升级" },
  { id: "dsh-crashed", label: "意外退出" },
  { id: "toast-latest", label: "toast:已是最新" },
  { id: "toast-error", label: "toast:检查失败" },
]

function readParams(): { platform: Platform; scene: SceneId; menuOpen: boolean } {
  const p = new URLSearchParams(window.location.search)
  return {
    platform: (p.get("platform") ?? "macos") as Platform,
    scene: (p.get("scene") ?? "normal") as SceneId,
    menuOpen: p.get("menu") === "open",
  }
}

/** 改单个查询参数并 replaceState(不产生历史记录,URL 始终可分享)。 */
function writeParam(key: string, value: string | null) {
  const p = new URLSearchParams(window.location.search)
  if (value === null) p.delete(key)
  else p.set(key, value)
  window.history.replaceState(null, "", `${window.location.pathname}?${p.toString()}`)
}

// ---------- mock 菜单快照(#33:MenuSnapshot 形状,快照构建是纯函数) ----------

type MenuKind = "action" | "check" | "separator" | "submenu"
interface MenuItem {
  id: string
  label?: string
  kind: MenuKind
  checked?: boolean
  disabled?: boolean
  /** 原型私货:#33 快照未定徽标字段——「动态升级条目」的徽标形态留待拍板。 */
  badge?: boolean
  children?: MenuItem[]
}

/** 菜单状态源(真实现归 Rust:theme.rs / autostart.rs / close_behavior)。 */
interface MenuState {
  theme: "light" | "dark" | "system"
  autostart: boolean
  dshTarget: string
  closeBehavior: "ask" | "minimize" | "quit"
}

/** 快照构建纯函数(生产路径:Rust 同构逻辑,此处 mock)。 */
function buildMenuSnapshot(s: MenuState): MenuItem[] {
  const closeBehaviorLabels: Record<MenuState["closeBehavior"], string> = {
    ask: "每次询问",
    minimize: "最小化",
    quit: "退出",
  }
  return [
    { id: "toggle", kind: "action", label: "隐藏窗口" },
    {
      id: "theme",
      kind: "submenu",
      label: "主题",
      children: [
        { id: "theme-light", kind: "check", label: "亮色", checked: s.theme === "light" },
        { id: "theme-dark", kind: "check", label: "暗色", checked: s.theme === "dark" },
        { id: "theme-system", kind: "check", label: "跟随系统", checked: s.theme === "system" },
      ],
    },
    { id: "sep-1", kind: "separator" },
    { id: "autostart", kind: "check", label: "开机自启", checked: s.autostart },
    { id: "upgrade-dsh", kind: "action", label: `升级 dsh 到 ${s.dshTarget}`, badge: true },
    { id: "check-update", kind: "action", label: "检查更新" },
    { id: "sep-2", kind: "separator" },
    {
      id: "settings",
      kind: "submenu",
      label: "设置",
      children: [
        {
          id: "close-behavior",
          kind: "submenu",
          label: "关闭行为",
          children: (["ask", "minimize", "quit"] as const).map((v) => ({
            id: `close-${v}`,
            kind: "check",
            label: closeBehaviorLabels[v],
            checked: s.closeBehavior === v,
          })),
        },
      ],
    },
    { id: "quit", kind: "action", label: "退出" },
  ]
}

// ---------- 壳菜单条(三平台同一组件;#28:macOS 拖拽条内,Win/Linux 内容区第一行) ----------

function ShellMenuBar({
  platform,
  snapshot,
  onAction,
  upgradeBadge,
  menuOpen,
  onOpenChange,
}: {
  platform: Platform
  snapshot: MenuItem[]
  onAction: (id: string) => void
  /** 应用有可用更新时菜单按钮亮徽标点(#31 自动检查命中不打扰)。 */
  upgradeBadge: boolean
  menuOpen: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <div
      // #28:macOS 整行 28px 拖拽条(data-tauri-drag-region 真实现用;控件自身豁免)
      data-tauri-drag-region={platform === "macos" ? true : undefined}
      title={platform === "macos" ? "28px 拖拽条 + 菜单条同行(macOS Overlay)" : undefined}
      className={cn(
        "relative z-30 flex items-center",
        platform === "macos" ? "h-7 pl-[84px]" : "h-9 border-b border-border px-3",
      )}
    >
      {platform === "macos" && (
        // 系统红绿灯占位示意(Overlay 保留系统控件,左侧 ~13px 起)
        <div className="absolute top-1/2 left-3.5 flex -translate-y-1/2 items-center gap-2">
          <span className="size-3 rounded-full bg-[#ff5f57]" title="关闭(点击演示关闭三选)" />
          <span className="size-3 rounded-full bg-[#febc2e]" title="最小化(示意)" />
          <span className="size-3 rounded-full bg-[#28c840]" title="缩放(示意)" />
        </div>
      )}
      <DropdownMenu open={menuOpen} onOpenChange={onOpenChange}>
        <DropdownMenuTrigger
          render={
            <Button variant="ghost" size="sm" className="relative h-6 gap-1.5 px-2 text-xs">
              <MenuIcon className="size-3.5" />
              菜单
              {upgradeBadge && (
                <span className="absolute top-0.5 right-0.5 size-2 rounded-full bg-primary" title="有可用更新" />
              )}
            </Button>
          }
        />
        <DropdownMenuContent align="start" className="w-56">
          {snapshot.map((item) => (
            <SnapshotItem key={item.id} item={item} onAction={onAction} />
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <span className="ml-auto pr-3 text-[10px] text-muted-foreground/60 select-none">
        {platform === "macos" ? "拖拽区示意(#28)" : "壳菜单条 · 内容区第一行(#28)"}
      </span>
    </div>
  )
}

/** 快照纯映射渲染(#33:check 勾选 / disabled 禁用 / submenu 递归)。 */
function SnapshotItem({
  item,
  onAction,
}: {
  item: MenuItem
  onAction: (id: string) => void
}) {
  switch (item.kind) {
    case "separator":
      return <DropdownMenuSeparator />
    case "submenu":
      return (
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>{item.label}</DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            {item.children?.map((child) => (
              <SnapshotItem key={child.id} item={child} onAction={onAction} />
            ))}
          </DropdownMenuSubContent>
        </DropdownMenuSub>
      )
    case "check":
      return (
        <DropdownMenuCheckboxItem
          checked={item.checked}
          onCheckedChange={() => onAction(item.id)}
        >
          {item.label}
        </DropdownMenuCheckboxItem>
      )
    default:
      return (
        <DropdownMenuItem disabled={item.disabled} onClick={() => onAction(item.id)}>
          {item.label}
          {item.badge && (
            <span className="ml-auto rounded-full bg-primary/15 px-1.5 py-0.5 text-[10px] font-medium text-primary">
              新
            </span>
          )}
        </DropdownMenuItem>
      )
  }
}

// ---------- 系统标题栏示意(Windows / Linux;真实现为系统原生,应用无控制点) ----------

function SystemTitleBar({ platform, onClose }: { platform: "windows" | "linux"; onClose: () => void }) {
  if (platform === "windows") {
    return (
      <div className="flex h-8 shrink-0 items-center bg-neutral-200/80 dark:bg-neutral-800">
        <img src="/whale.svg" alt="" className="ml-3 size-4" />
        <span className="ml-2 text-xs text-neutral-700 dark:text-neutral-300">
          DeepSeek Desktop <span className="opacity-50">(系统标题栏示意)</span>
        </span>
        <div className="ml-auto flex h-full">
          <button type="button" className="flex h-full w-11 items-center justify-center hover:bg-neutral-400/40" title="最小化(示意)">
            <Minus className="size-3.5" />
          </button>
          <button type="button" className="flex h-full w-11 items-center justify-center hover:bg-neutral-400/40" title="最大化(示意)">
            <Square className="size-3" />
          </button>
          <button
            type="button"
            onClick={onClose}
            className="flex h-full w-11 items-center justify-center hover:bg-[#c42b1c] hover:text-white"
            title="关闭 → 演示关闭三选"
          >
            <X className="size-4" />
          </button>
        </div>
      </div>
    )
  }
  return (
    <div className="relative flex h-8 shrink-0 items-center border-b border-border bg-background">
      <span className="absolute inset-x-0 text-center text-xs text-muted-foreground">
        DeepSeek Desktop <span className="opacity-50">(系统标题栏示意 · 外观由 GTK 主题决定)</span>
      </span>
      <div className="ml-auto flex items-center gap-1 pr-2">
        <button type="button" className="flex size-7 items-center justify-center rounded-full hover:bg-muted" title="最小化(示意)">
          <Minus className="size-3.5" />
        </button>
        <button type="button" className="flex size-7 items-center justify-center rounded-full hover:bg-muted" title="最大化(示意)">
          <Square className="size-3" />
        </button>
        <button
          type="button"
          onClick={onClose}
          className="flex size-7 items-center justify-center rounded-full hover:bg-muted"
          title="关闭 → 演示关闭三选"
        >
          <X className="size-3.5" />
        </button>
      </div>
    </div>
  )
}

// ---------- dsh 占位与全屏覆盖层(#32:互斥,盖 iframe 区域,菜单条常驻) ----------

function DshPlaceholder() {
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-muted text-muted-foreground">
      <Globe className="size-8 opacity-40" />
      <p className="text-sm font-medium">dsh Web UI 占位</p>
      <p className="text-xs opacity-70">实际此处为加载 dsh 的 iframe(本票不动真 iframe)</p>
    </div>
  )
}

/** boot 启动过渡:视觉沿用 BootScreen 的仪表环 + 不确定进度。 */
function BootOverlay() {
  return (
    <div className="absolute inset-0 z-20 flex items-center justify-center bg-background">
      <div className="flex w-full max-w-lg items-center gap-8 px-6">
        <div className="relative size-[124px] shrink-0" aria-hidden>
          <div className="absolute inset-0 rounded-full border-[3px] border-primary/10" />
          <div className="absolute inset-0 animate-spin rounded-full border-[3px] border-transparent border-t-primary" />
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-6">
          <div>
            <h1 className="text-lg font-medium">正在启动 dsh…</h1>
            <p className="mt-1.5 text-sm text-muted-foreground">首次启动需要初始化环境,可能需要一两分钟</p>
          </div>
          <ProgressRail value={null} />
        </div>
      </div>
    </div>
  )
}

/** dsh 升级流水线:四阶段步进 + 安装进度(一次截图像看到全部阶段词汇)。 */
function DshUpgradeOverlay() {
  const stages: { key: string; label: string }[] = [
    { key: "killing", label: "停止旧版服务" },
    { key: "installing", label: "安装新版本" },
    { key: "verifying", label: "校验安装" },
    { key: "starting", label: "重启 dsh" },
  ]
  const current = "installing"
  const currentIndex = stages.findIndex((s) => s.key === current)
  return (
    <div className="absolute inset-0 z-20 flex items-center justify-center bg-background">
      <div className="flex w-full max-w-lg flex-col gap-6 px-6">
        <div className="flex items-center gap-4">
          <Loader2 className="size-9 shrink-0 animate-spin text-primary" />
          <div>
            <h1 className="text-lg font-medium">正在升级 dsh…</h1>
            <p className="mt-1 text-sm text-muted-foreground">v0.1.3 → v0.2.0 · 升级可能需要几分钟</p>
          </div>
          <span className="ml-auto rounded-full bg-muted px-2.5 py-1 font-mono text-[10px] text-muted-foreground">
            phase: {current}
          </span>
        </div>
        <ol className="flex flex-col gap-2.5">
          {stages.map((s, i) => {
            const state = i < currentIndex ? "done" : i === currentIndex ? "current" : "pending"
            return (
              <li key={s.key} className="flex items-center gap-2.5 text-sm">
                {state === "done" && (
                  <span className="flex size-4 items-center justify-center rounded-full bg-primary text-primary-foreground">
                    <svg viewBox="0 0 12 12" className="size-2.5" aria-hidden>
                      <path d="M2 6l2.5 2.5L10 3.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
                    </svg>
                  </span>
                )}
                {state === "current" && <Loader2 className="size-4 animate-spin text-primary" />}
                {state === "pending" && <span className="size-4 rounded-full border-2 border-muted-foreground/30" />}
                <span className={state === "pending" ? "text-muted-foreground/50" : state === "current" ? "font-medium" : "text-muted-foreground"}>
                  {s.label}
                </span>
              </li>
            )
          })}
        </ol>
        <div className="flex flex-col gap-1.5">
          <ProgressRail value={62} />
          <p className="text-right text-xs tabular-nums text-muted-foreground">62%</p>
        </div>
      </div>
    </div>
  )
}

/** dsh 意外退出错误态:ErrorScreen 形态 + [重试](重跑 boot)。 */
function DshCrashedOverlay({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-5 bg-background text-center">
      <CircleAlert className="size-9 text-destructive" />
      <div className="flex max-w-md flex-col items-center gap-2">
        <h1 className="text-lg font-medium">dsh 意外退出</h1>
        <p className="text-sm leading-relaxed text-muted-foreground">
          dsh 服务进程意外退出(退出码 1),页面已不可用。你可以重试启动;数据保存在本机,不受影响。
        </p>
      </div>
      <div className="flex items-center gap-3">
        <Button size="lg" onClick={onRetry}>
          <RotateCw />
          重试
        </Button>
        <Button variant="ghost" size="lg">
          退出
        </Button>
      </div>
    </div>
  )
}

/** 应用更新进度:右下角非模态浮层(#31:下载不打断使用 dsh,完成后变重启决策)。 */
function UpdateFloat({
  mode,
  onRestart,
  onLater,
}: {
  mode: "downloading" | "ready"
  onRestart: () => void
  onLater: () => void
}) {
  return (
    <div className="absolute right-4 bottom-4 z-30 flex w-80 flex-col gap-3 rounded-2xl border border-border bg-popover/95 p-4 shadow-xl backdrop-blur">
      {mode === "downloading" ? (
        <>
          <div className="flex items-center gap-2.5 text-sm font-medium">
            <Loader2 className="size-4 animate-spin text-primary" />
            正在下载更新 v0.5.0…
          </div>
          <div className="flex flex-col gap-1">
            <ProgressRail value={42} />
            <p className="text-right text-xs tabular-nums text-muted-foreground">已下载 42%</p>
          </div>
        </>
      ) : (
        <>
          <div className="flex items-center gap-2.5 text-sm font-medium">
            <PartyPopper className="size-4 text-primary" />
            更新已就绪
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">
            重启应用以完成安装。重启后 dsh 服务随之重启,数据不受影响。
          </p>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={onLater}>
              稍后
            </Button>
            <Button size="sm" onClick={onRestart}>
              <RotateCw />
              立即重启
            </Button>
          </div>
        </>
      )}
    </div>
  )
}

// ---------- 原型工具条(平台 × 场景切换;非被评设计的一部分) ----------

function PrototypeToolbar({
  platform,
  scene,
  onPlatform,
  onScene,
}: {
  platform: Platform
  scene: SceneId
  onPlatform: (p: Platform) => void
  onScene: (s: SceneId) => void
}) {
  return (
    <div className="fixed inset-x-0 top-0 z-[60] flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-dashed border-border bg-card/95 px-4 py-2 backdrop-blur">
      <span className="rounded-full bg-destructive/15 px-2 py-0.5 text-[10px] font-semibold tracking-wide text-destructive">
        PROTOTYPE #30 · throwaway
      </span>
      <div className="flex items-center gap-1">
        {PLATFORMS.map((p) => (
          <Button
            key={p.id}
            size="xs"
            variant={p.id === platform ? "secondary" : "ghost"}
            onClick={() => onPlatform(p.id)}
          >
            {p.label}
          </Button>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-1">
        {SCENES.map((s) => (
          <Button
            key={s.id}
            size="xs"
            variant={s.id === scene ? "secondary" : "ghost"}
            onClick={() => onScene(s.id)}
          >
            {s.label}
          </Button>
        ))}
      </div>
      <span className="ml-auto text-[10px] text-muted-foreground select-none">
        ←/→ 切换场景 · URL 可直接分享
      </span>
    </div>
  )
}

// ---------- 组装 ----------

export default function ShellPrototype() {
  const initial = useMemo(readParams, [])
  const [platform, setPlatform] = useState<Platform>(initial.platform)
  const [scene, setScene] = useState<SceneId>(initial.scene)
  const [menuOpen, setMenuOpen] = useState(initial.menuOpen)
  const [rememberClose, setRememberClose] = useState(false)
  // 暗色为默认语境;主题菜单项可真切换(模拟 Rust refresh_menu 重发快照)
  const [menuState, setMenuState] = useState<MenuState>({
    theme: "dark",
    autostart: true,
    dshTarget: "0.2.0",
    closeBehavior: "ask",
  })

  // 暗色默认语境(真实现由 Rust 经 useThemeSync 驱动)
  useEffect(() => {
    document.documentElement.classList.toggle("dark", menuState.theme !== "light")
  }, [menuState.theme])

  // toast 场景:进场即弹(#31 已是最新 / 检查失败 = toast,不弹窗打断)
  useEffect(() => {
    if (scene === "toast-latest") {
      toast.success("已是最新版本:应用 v0.4.0 · dsh v0.1.3")
    } else if (scene === "toast-error") {
      toast.error("检查更新失败:网络连接超时,请稍后重试")
    }
  }, [scene])

  const snapshot = useMemo(() => buildMenuSnapshot(menuState), [menuState])
  const upgradeAvailable = scene === "update-found" || scene === "update-downloading" || scene === "update-ready"

  const setSceneAndUrl = (s: SceneId) => {
    setScene(s)
    writeParam("scene", s === "normal" ? null : s)
  }
  const setPlatformAndUrl = (p: Platform) => {
    setPlatform(p)
    writeParam("platform", p === "macos" ? null : p)
  }

  /** 动作分发(#33:单一动作表——真实现 Rust menu_action,原型本地 mock)。 */
  const handleMenuAction = (id: string) => {
    switch (id) {
      case "theme-light":
      case "theme-dark":
      case "theme-system":
        setMenuState((s) => ({ ...s, theme: id.slice(6) as MenuState["theme"] }))
        break
      case "autostart":
        setMenuState((s) => ({ ...s, autostart: !s.autostart }))
        break
      case "close-ask":
      case "close-minimize":
      case "close-quit":
        setMenuState((s) => ({ ...s, closeBehavior: id.slice(6) as MenuState["closeBehavior"] }))
        break
      case "upgrade-dsh":
        setMenuOpen(false)
        writeParam("menu", null)
        setSceneAndUrl("dsh-upgrade")
        break
      case "check-update":
        toast.success("已是最新版本:应用 v0.4.0 · dsh v0.1.3")
        break
      case "toggle":
        toast.info("窗口已隐藏到托盘(示意)")
        break
      case "quit":
        toast.info("退出应用(示意)")
        break
    }
  }

  // ←/→ 键切换场景(输入框聚焦时不拦截)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return
      }
      const i = SCENES.findIndex((s) => s.id === scene)
      if (e.key === "ArrowRight") setSceneAndUrl(SCENES[(i + 1) % SCENES.length].id)
      if (e.key === "ArrowLeft") setSceneAndUrl(SCENES[(i - 1 + SCENES.length) % SCENES.length].id)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [scene])

  const isMac = platform === "macos"

  return (
    <div className="min-h-screen bg-neutral-900 font-sans text-foreground">
      <PrototypeToolbar
        platform={platform}
        scene={scene}
        onPlatform={setPlatformAndUrl}
        onScene={setSceneAndUrl}
      />

      {/* 桌面背景示意:窗口外的中性底,只为衬托三平台窗口外框差异 */}
      <main className="flex min-h-screen items-center justify-center p-6 pt-16">
        <div className="pointer-events-none fixed bottom-2 left-3 text-[10px] text-neutral-500 select-none">
          桌面背景(示意)
        </div>
        {/* 应用窗口示意:macOS 无边框圆角;Win/Linux 带系统标题栏 */}
        <div
          className={cn(
            "relative flex h-[680px] max-h-[calc(100vh-7rem)] w-[1100px] max-w-[calc(100vw-5rem)] flex-col overflow-hidden bg-background shadow-2xl",
            isMac ? "rounded-2xl" : "rounded-lg",
          )}
        >
          {!isMac && <SystemTitleBar platform={platform} onClose={() => setSceneAndUrl("close-ask")} />}
          {isMac && (
            // macOS 红绿灯可点:红灯 → 关闭三选
            <button
              type="button"
              aria-label="关闭窗口(演示关闭三选)"
              onClick={() => setSceneAndUrl("close-ask")}
              className="absolute top-4 left-3.5 z-40 size-3 rounded-full bg-[#ff5f57] hover:brightness-110"
            />
          )}
          <ShellMenuBar
            platform={platform}
            snapshot={snapshot}
            onAction={handleMenuAction}
            upgradeBadge={upgradeAvailable}
            menuOpen={menuOpen}
            onOpenChange={(open) => {
              setMenuOpen(open)
              writeParam("menu", open ? "open" : null)
            }}
          />
          {/* iframe 区域 + 覆盖层(#32:覆盖层盖此区域,菜单条常驻) */}
          <div className="relative flex-1 overflow-hidden">
            <DshPlaceholder />
            {scene === "boot" && <BootOverlay />}
            {scene === "dsh-upgrade" && <DshUpgradeOverlay />}
            {scene === "dsh-crashed" && (
              <DshCrashedOverlay onRetry={() => setSceneAndUrl("boot")} />
            )}
            {scene === "update-downloading" && (
              <UpdateFloat mode="downloading" onRestart={() => {}} onLater={() => {}} />
            )}
            {scene === "update-ready" && (
              <UpdateFloat mode="ready" onRestart={() => toast.info("重启应用(示意)")} onLater={() => setSceneAndUrl("normal")} />
            )}
          </div>
        </div>
      </main>

      {/* 弹窗(#31:AlertDialog 盖整个窗口;shadcn 默认全屏模态) */}
      <AlertDialog open={scene === "update-found"} onOpenChange={(o) => !o && setSceneAndUrl("normal")}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogMedia>
              <CircleArrowUp className="text-primary" />
            </AlertDialogMedia>
            <AlertDialogTitle>发现新版本 v0.5.0</AlertDialogTitle>
            <AlertDialogDescription className="whitespace-pre-line text-left">
              {"当前 v0.4.0 · 更新内容:\n· 壳菜单条与三平台窗口控制(#28)\n· 六弹窗与更新提示交互(#31)\n· dsh 升级覆盖层编排(#32)"}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel variant="ghost" onClick={() => setSceneAndUrl("normal")}>
              稍后
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => setSceneAndUrl("update-downloading")}>
              <CircleArrowUp />
              升级
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={scene === "close-ask"} onOpenChange={(o) => !o && setSceneAndUrl("normal")}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>关闭 DeepSeek Desktop?</AlertDialogTitle>
            <AlertDialogDescription>
              最小化到托盘可保持 dsh 后台运行;退出将同时停止 dsh 服务。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <label className="-mt-3 flex cursor-pointer items-center gap-2.5 text-sm text-muted-foreground">
            <Checkbox checked={rememberClose} onCheckedChange={(v) => setRememberClose(v === true)} />
            记住我的选择(设置 ▸ 关闭行为)
          </label>
          <AlertDialogFooter>
            <AlertDialogCancel variant="ghost" onClick={() => setSceneAndUrl("normal")}>
              取消
            </AlertDialogCancel>
            <AlertDialogAction variant="outline" onClick={() => { toast.info("退出应用(示意)"); setSceneAndUrl("normal") }}>
              退出
            </AlertDialogAction>
            <AlertDialogAction onClick={() => { toast.info("已最小化到托盘(示意)"); setSceneAndUrl("normal") }}>
              最小化到托盘
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <Toaster position="bottom-right" offset={24} />
    </div>
  )
}
