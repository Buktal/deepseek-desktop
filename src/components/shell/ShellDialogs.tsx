// 壳页弹窗渲染(#31 拍板 / #39 施工):Rust `shell-dialog` 事件 → 按 kind 分派
// 渲染 AlertDialog 或 toast。机制与菜单快照同构——文案全部随载荷(Rust
// locales 解析下发),本组件零文案表、零翻译;用户选择经 `shell_dialog_respond`
// 回流 Rust 统一动作分发(tray.rs,与 menu_action 同一张表)。
//
// kind 分派(#31 六弹窗):
// - dialog 类(update-found / upgrade-found / close-ask):AlertDialog,
//   按钮次序与强调随 payload(疑点 3 结论);初始焦点落主按钮(Enter 即默认
//   动作),Esc/遮罩点击 = 取消(close-ask 复位防双触发守卫;found 类在
//   Rust 侧与「稍后」同无操作)
// - toast 类(toast-up-to-date / toast-check-failed / toast-upgrade-running):
//   Sonner toast,信息性无决策,无需 respond
//
// close-ask 无取消按钮(遮罩点击/Esc 即取消语义,窗口保持现状)。
// 同一时刻只保留一个弹窗:新请求覆盖旧请求(弹窗请求低频,Rust 侧编排互斥)。
import { invoke } from "@tauri-apps/api/core"
import { CircleArrowUp, TriangleAlert } from "lucide-react"
import { useRef, useState } from "react"
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
import { useRustEvent } from "@/lib/useRustEvent"
import {
  isCheckUpdateAnswer,
  isShellDialogRequest,
  type DialogButtonVariant,
  type ShellDialogRequest,
} from "@/lib/shellDialog"
import { dismissCheckUpdateLoading } from "@/lib/updateCheckToast"

/** 载荷 variant → Button variant(疑点 3:primary/outline/ghost 三种强调)。 */
const BUTTON_VARIANT: Record<DialogButtonVariant, "default" | "outline" | "ghost"> = {
  primary: "default",
  outline: "outline",
  ghost: "ghost",
}

export function ShellDialogs() {
  const [request, setRequest] = useState<ShellDialogRequest | null>(null)
  // 关闭弹窗的「记住我的选择」:勾选状态只在弹窗存活期间有效,
  // 新请求到达时复位(勾选本身不持久化——持久化在 Rust close::set,#31)
  const [remember, setRemember] = useState(false)
  // 初始焦点目标:主按钮(无 primary 时退首个),Enter 直接执行默认动作
  const primaryRef = useRef<HTMLButtonElement>(null)

  // 监听 shell-dialog 事件(一次性请求,无快照命令;样板走 useRustEvent)。
  // Rust 侧 emit 前已 show 窗口,事件不丢;toast 类在此分流(Sonner toast,
  // 渲染路径不再出现 toast 请求)
  useRustEvent(
    "shell-dialog",
    (req) => {
      // 检查更新结果到达:关掉手动检查的在途 loading toast(五种回答形态;
      // close-ask 等无关事件不动它,见 shellDialog.isCheckUpdateAnswer)
      if (isCheckUpdateAnswer(req.kind)) dismissCheckUpdateLoading()
      if (req.kind.startsWith("toast-")) {
        toast(req.message ?? "")
      } else {
        setRequest(req)
        setRemember(false)
      }
    },
    isShellDialogRequest,
  )

  // 用户选择:respond 回流 Rust 统一分发(关闭弹窗的记住勾选一并回传);
  // 未知 kind 由 Rust 侧无操作兜底
  const respond = (choice: string) => {
    if (!request) return
    void invoke("shell_dialog_respond", { kind: request.kind, choice, remember }).catch(
      (e) => console.warn("[dialog] 弹窗回答失败", e),
    )
    setRequest(null)
  }

  // toast 类请求已在监听器里分流(Sonner toast 直接弹出),渲染路径只可能
  // 承载 dialog 类请求(Q4:此处曾有一段恒假的 isToast 死分支)
  if (!request) return null

  const primaryId =
    request.buttons.find((b) => b.variant === "primary")?.id ?? request.buttons[0]?.id

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        // Esc:取消语义(close-ask 复位防双触发守卫;found 类无操作)
        if (!open) respond("cancel")
      }}
    >
      <AlertDialogContent
        size={request.kind === "close-ask" ? "sm" : "default"}
        // 遮罩点击:同 Esc 取消语义(Base UI AlertDialog 默认不因外点自关)
        onOutsideClick={() => respond("cancel")}
        initialFocus={primaryRef}
      >
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
              ref={button.id === primaryId ? primaryRef : undefined}
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
