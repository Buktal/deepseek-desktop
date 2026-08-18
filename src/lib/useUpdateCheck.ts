// 应用自身升级的前端接线(#5):镜像 Rust 侧升级状态 + 卡片动作。
//
// 架构(#9 重审定稿):本外壳的 dsh 页是 remote origin,收不到任何 Tauri 事件/命令,
// O_CC_One 的前端常驻 update-check hook(启动探测 + 6h 轮询)不适用——检查/下载/
// 安装全部在 Rust 侧(update.rs:启动探测 + 6h 轮询 + 托盘手动入口),
// 本 hook 只是本地页上的镜像视图:挂载时拉 `update_state` 快照 + 监听
// `update-state` 事件,动作全部走命令(update_apply / update_restart /
// update_dismiss / opener 打开 GitHub Releases)。
//
// 升级卡镜像基建(F1):requested 生命周期 / card-request 监听走
// useTrayCardMirror 共享实现(useDshUpgrade 同款),本 hook 只声明差异面:
// 字段映射(apply)、快照命令与动作命令表;卡片可见性由 deriveOverlay 内部
// 策略判定(F4),本 hook 暴露原始输入(status + requested)。
//
// 错误跨边界保持结构化形态({kind,data}),卡片渲染时才经 localizeStructuredError
// 翻译(语言切换可重译,见 src/lib/error.ts 与 #12)。

import { invoke } from "@tauri-apps/api/core"
import { openUrl } from "@tauri-apps/plugin-opener"
import { useCallback, useState } from "react"

import { toStructuredError, type RustErrorPayload, type StructuredError } from "@/lib/error"
import { RELEASES_URL } from "@/lib/updateShare"
import { useTrayCardMirror } from "@/lib/useTrayCardMirror"

export type UpdateStatus =
  | "idle" // 无更新 / 检测失败(静默)
  | "checking" // 检查在途(卡片不渲染此态)
  | "available" // 发现新版,等待用户操作
  | "downloading" // 下载/安装中
  | "ready" // 已安装,等待重启
  | "failed" // 下载/安装失败 → 降级 GitHub 手动下载

/** Rust 侧 UpdateStateView 的序列化形态(内部 tag status,字段 camelCase) */
export interface UpdateStateView {
  status: UpdateStatus
  version?: string | null
  currentVersion?: string | null
  notes?: string | null
  downloadedBytes?: number
  totalBytes?: number
  error?: RustErrorPayload | null
}

export function useUpdateCheck() {
  // 跨边界保持结构化形态,卡片渲染时才翻译(语言切换不冻结旧文案)
  const [error, setError] = useState<StructuredError | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const [currentVersion, setCurrentVersion] = useState<string | null>(null)
  const [notes, setNotes] = useState<string | null>(null)
  const [downloadedBytes, setDownloadedBytes] = useState(0)
  const [totalBytes, setTotalBytes] = useState(0)

  const mirror = useTrayCardMirror({
    stateEvent: "update-state",
    cardRequestEvent: "update-card-request",
    snapshot: () => invoke<UpdateStateView>("update_state"),
    // status/requested 由 mirror 接管,这里只写其余字段(事件与快照同构,
    // 后到者覆盖,无竞态)
    apply: (view) => {
      setVersion(view.version ?? null)
      setCurrentVersion(view.currentVersion ?? null)
      setNotes(view.notes ?? null)
      setDownloadedBytes(view.downloadedBytes ?? 0)
      setTotalBytes(view.totalBytes ?? 0)
      setError(toStructuredError(view.error ?? null))
    },
    actions: {
      applyUpdate: "update_apply",
      restartNow: "update_restart",
      dismiss: "update_dismiss",
    },
    // 监听/快照失败:保持 idle,升级卡片不出现(升级是增强,不是硬依赖)
    onError: (e) => console.warn("[update] 升级状态同步失败,保持 idle", e),
  })

  // 非命令动作:失败降级入口经 opener 开系统浏览器(Rust 命令表之外)
  const openReleases = useCallback(() => {
    void openUrl(RELEASES_URL).catch(() => {})
  }, [])

  return {
    ...mirror,
    error,
    version,
    currentVersion,
    notes,
    downloadedBytes,
    totalBytes,
    openReleases,
  }
}
