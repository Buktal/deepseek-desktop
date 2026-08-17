// 壳页弹窗渲染(#31 拍板 / #39 施工):Rust `shell-dialog` 事件 → 按 kind 分派
// 渲染 AlertDialog 或 toast。机制与菜单快照同构——文案全部随载荷(Rust
// locales 解析下发),本组件零文案表、零翻译;用户选择经 `shell_dialog_respond`
// 回流 Rust 统一动作分发(tray.rs,与 menu_action 同一张表)。
//
// kind 分派(#31 六弹窗):
// - dialog 类(update-found / upgrade-found / close-ask):AlertDialog,
//   按钮次序与强调随 payload(疑点 3 结论);Esc/遮罩 = 取消(later 语义同源)
// - toast 类(toast-up-to-date / toast-check-failed / toast-upgrade-running):
//   Sonner toast,信息性无决策,无需 respond
//
// 同一时刻只保留一个弹窗:新请求覆盖旧请求(弹窗请求低频,Rust 侧编排互斥)。
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { CircleArrowUp, TriangleAlert } from "lucide-react"
import { useEffect, useState } from "react"
import { toast } from "sonner"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Checkbox } from "@/components/ui/checkbox"
import { summarizeReleaseNotes } from "@/lib/releaseNotes"
import {
  isShellDialogRequest,
  type DialogButtonVariant,
  type ShellDialogRequest,
} from "@/lib/shellDialog"

/** 载荷 variant → Button variant(疑点 3:primary/outline/ghost 三种强调)。 */
const BUTTON_VARIANT: Record<DialogButtonVariant, "default" | "outline" | "ghost"> = {
  primary: "default",
  outline: "outline",
  ghost: "ghost",
}

export function ShellDialogs() {
  const [request, setRequest] = useState<ShellDialogRequest | null>(null)
  // 关闭三选的「记住我的选择」:勾选状态只在弹窗存活期间有效,
  // 新请求到达时复位(勾选本身不持久化——持久化在 Rust close::set,#31)
  const [remember, setRemember] = useState(false)

  // 监听 shell-dialog 事件(一次性请求,无快照命令;与 upgrade-card-request
  // 同款 listen 模式)。Rust 侧 emit 前已 show 窗口,事件不丢。
  useEffect(() => {
    let alive = true
    let stop: (() => void) | undefined
    void listen("shell-dialog", (e) => {
      if (!alive) return
      const req = e.payload
      if (!isShellDialogRequest(req)) return
      if (req.kind.startsWith("toast-")) {
        toast(req.message ?? "")
      } else {
        setRequest(req)
        setRemember(false)
      }
    }).then((un) => {
      stop = un
      if (!alive) un()
    })
    return () => {
      alive = false
      stop?.()
    }
  }, [])

  // 用户选择:respond 回流 Rust 统一分发(关闭三选的记住勾选一并回传);
  // 未知 kind 由 Rust 侧无操作兜底
  const respond = (choice: string) => {
    if (!request) return
    void invoke("shell_dialog_respond", { kind: request.kind, choice, remember }).catch(
      (e) => console.warn("[dialog] 弹窗回答失败", e),
    )
    setRequest(null)
  }

  if (!request) return null

  const isToast = request.kind.startsWith("toast-")
  if (isToast) return null

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        // Esc / 遮罩点击:取消语义(close-ask 复位防双触发守卫;found 类无操作)
        if (!open) respond("cancel")
      }}
    >
      <AlertDialogContent size={request.kind === "close-ask" ? "sm" : "default"}>
        {request.kind === "close-ask" ? (
          <>
            <AlertDialogHeader>
              {request.title ? <AlertDialogTitle>{request.title}</AlertDialogTitle> : null}
            </AlertDialogHeader>
            {/* 记住勾选:勾选后所选去向(最小化/退出)持久化,下次直接执行不再弹(#31) */}
            <label className="flex items-center gap-2 text-sm text-muted-foreground">
              <Checkbox checked={remember} onCheckedChange={setRemember} />
              {request.rememberLabel}
            </label>
          </>
        ) : (
          <AlertDialogHeader>
            {/* 升级类弹窗的语义图标(Media 插槽,default 尺寸下与标题同行) */}
            <AlertDialogMedia>
              {request.kind === "upgrade-found" ? (
                <TriangleAlert className="size-8 text-primary" />
              ) : (
                <CircleArrowUp className="size-8 text-primary" />
              )}
            </AlertDialogMedia>
            {request.title ? <AlertDialogTitle>{request.title}</AlertDialogTitle> : null}
            {request.message ? (
              <AlertDialogDescription>{request.message}</AlertDialogDescription>
            ) : null}
            {/* release notes 摘要(前端复用 summarizeReleaseNotes,与 UpdateCard 同款) */}
            {request.notes ? (
              <AlertDialogDescription className="max-h-28 max-w-sm overflow-y-auto whitespace-pre-line break-words">
                {summarizeReleaseNotes(request.notes)}
              </AlertDialogDescription>
            ) : null}
          </AlertDialogHeader>
        )}
        <AlertDialogFooter>
          {request.buttons.map((button) => (
            <AlertDialogAction
              key={button.id}
              variant={BUTTON_VARIANT[button.variant]}
              onClick={() => respond(button.id)}
            >
              {button.label}
            </AlertDialogAction>
          ))}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
