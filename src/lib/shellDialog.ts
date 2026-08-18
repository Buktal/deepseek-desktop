// 壳页弹窗请求的线上契约(#31 拍板 / #39 施工):Rust `shell-dialog` 事件载荷。
// 与菜单快照同构——文案由 Rust locales 解析后放进载荷(Rust 一处,前端零
// 第二份文案表);本模块只提供形状守卫,供 ShellDialogs 组件过滤未知载荷。
// 按钮 id 是 Rust 动作表的唯一事实源(respond 原样回传),次序即视觉次序,
// 强调随按钮下发(疑点 3 结论)。

export type ShellDialogKind =
  | "update-found" // 发现应用新版(AlertDialog,notes 承载 release notes 原文)
  | "upgrade-found" // 发现 dsh 新版(AlertDialog)
  | "close-ask" // 关闭二选(AlertDialog + 记住勾选;遮罩点击/Esc = 取消)
  | "toast-up-to-date" // 已是最新(toast)
  | "toast-check-failed" // 检查失败(toast)
  | "toast-upgrade-running" // 升级流水线在途,手动检查被拒(toast)

/** 「检查更新」的回答形态:手动检查(tray.rs on_check_update)的结果必为
 *  这五种事件之一——两种 found 弹窗 + 三种 toast;close-ask 等无关事件不算。 */
const CHECK_UPDATE_ANSWERS: ReadonlySet<string> = new Set([
  "update-found",
  "upgrade-found",
  "toast-up-to-date",
  "toast-check-failed",
  "toast-upgrade-running",
])

/** shell-dialog 事件是否为「检查更新」的回答(手动检查在途 loading toast
 *  的关闭信号,见 updateCheckToast.ts)。 */
export function isCheckUpdateAnswer(kind: ShellDialogKind): boolean {
  return CHECK_UPDATE_ANSWERS.has(kind)
}

export type DialogButtonVariant = "primary" | "outline" | "ghost"

/** Rust 侧 DialogButton 的序列化形态 */
export interface ShellDialogButton {
  id: string
  label: string
  variant: DialogButtonVariant
}

/** Rust 侧 ShellDialogRequest 的序列化形态(字段 camelCase,可选字段缺省不出现) */
export interface ShellDialogRequest {
  kind: ShellDialogKind
  title?: string | null
  message?: string | null
  buttons: ShellDialogButton[]
  notes?: string | null
  rememberLabel?: string | null
}

/** 载荷形状守卫:shape 校验,未知值不应用到页面(与 isMenuSnapshot 同款)。 */
export function isShellDialogRequest(v: unknown): v is ShellDialogRequest {
  if (typeof v !== "object" || v === null) return false
  const req = v as Record<string, unknown>
  if (typeof req.kind !== "string") return false
  if (!Array.isArray(req.buttons)) return false
  if (req.title !== undefined && typeof req.title !== "string") return false
  if (req.message !== undefined && typeof req.message !== "string") return false
  if (req.notes !== undefined && typeof req.notes !== "string") return false
  if (req.rememberLabel !== undefined && typeof req.rememberLabel !== "string") {
    return false
  }
  return req.buttons.every(
    (b) =>
      typeof b === "object" &&
      b !== null &&
      typeof (b as Record<string, unknown>).id === "string" &&
      typeof (b as Record<string, unknown>).label === "string",
  )
}
