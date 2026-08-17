// 应用自身升级的前端接线(#5):镜像 Rust 侧升级状态 + 卡片动作。
//
// 架构(#9 重审定稿):本外壳的 dsh 页是 remote origin,收不到任何 Tauri 事件/命令,
// O_CC_One 的前端常驻 update-check hook(启动探测 + 6h 轮询)不适用——检查/下载/
// 安装全部在 Rust 侧(update.rs:启动探测 + 6h 轮询 + 托盘手动入口),
// 本 hook 只是本地页上的镜像视图:挂载时拉 `update_state` 快照 + 监听
// `update-state` 事件(同步骨架走 useRustStateSync,先注册监听再拉快照,
// 后到者覆盖),动作全部走命令(update_apply / update_restart / update_dismiss /
// opener 打开 GitHub Releases)。StrictMode 双挂载无害:监听注册/清理成对,
// 快照拉取幂等。
//
// 错误跨边界保持结构化形态({kind,data}),卡片渲染时才经 localizeStructuredError
// 翻译(语言切换可重译,见 src/lib/error.ts 与 #12)。

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { openUrl } from "@tauri-apps/plugin-opener"
import { useCallback, useEffect, useState } from "react"

import { toStructuredError, type RustErrorPayload, type StructuredError } from "@/lib/error"
import { useRustStateSync } from "@/lib/useRustStateSync"

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

/** GitHub Releases 手动下载入口(失败降级,照搬 O_CC_One 的 RELEASES_URL) */
export const RELEASES_URL = "https://github.com/Buktal/deepseek-desktop/releases/latest"

/**
 * 升级卡片是否可见(纯函数,可测)。#39 起卡片只承载两个决策面:
 * available 需显式请求(托盘「升级到 vX」菜单 → update-card-request 事件;
 * 自动检测只亮托盘徽标,不弹卡片,#3 §1);failed(失败降级 GitHub)必显。
 * downloading/ready 不渲染卡片——下载进度改由右下角非模态浮层 UpdateFloat
 * 呈现(下载不打断使用 dsh,#31 拍板)。
 */
export function isUpdateCardVisible(status: UpdateStatus, requested: boolean): boolean {
  return status === "available" ? requested : status === "failed"
}

/** 下载进度百分比(0-100)。total 未知(<=0)时返回 null,卡片显示「请稍候」。纯函数,可测。 */
export function updatePercent(downloadedBytes: number, totalBytes: number): number | null {
  if (totalBytes <= 0 || downloadedBytes < 0) return null
  return Math.min(100, Math.round((downloadedBytes / totalBytes) * 100))
}

export function useUpdateCheck() {
  const [status, setStatus] = useState<UpdateStatus>("idle")
  // 跨边界保持结构化形态,卡片渲染时才翻译(语言切换不冻结旧文案)
  const [error, setError] = useState<StructuredError | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const [currentVersion, setCurrentVersion] = useState<string | null>(null)
  const [notes, setNotes] = useState<string | null>(null)
  const [downloadedBytes, setDownloadedBytes] = useState(0)
  const [totalBytes, setTotalBytes] = useState(0)
  // 卡片显式请求:托盘「升级到 vX」点击(壳页常驻后无页面切换,available 态
  // 需此请求才弹卡;状态离开 available 即复位,不跨轮残留)
  const [requested, setRequested] = useState(false)

  // 托盘「升级到 vX」菜单 → 显示升级卡片(事件由 tray.rs 推送)
  useEffect(() => {
    let alive = true
    let stop: (() => void) | undefined
    void listen("update-card-request", () => {
      if (alive) setRequested(true)
    }).then((un) => {
      stop = un
      if (!alive) un()
    })
    return () => {
      alive = false
      stop?.()
    }
  }, [])

  // 事件与快照来自同一 Rust 状态,后到者覆盖,无竞态
  const applyView = useCallback((view: UpdateStateView) => {
    setStatus(view.status)
    setVersion(view.version ?? null)
    setCurrentVersion(view.currentVersion ?? null)
    setNotes(view.notes ?? null)
    setDownloadedBytes(view.downloadedBytes ?? 0)
    setTotalBytes(view.totalBytes ?? 0)
    setError(toStructuredError(view.error ?? null))
    // 请求只对 available 有意义:进入流水线/消费完毕(Idle)后复位,
    // 避免旧请求在下一轮自动检测命中时误弹卡片
    if (view.status !== "available") setRequested(false)
  }, [])

  useRustStateSync({
    event: "update-state",
    snapshot: () => invoke<UpdateStateView>("update_state"),
    apply: applyView,
    // 监听/快照失败:保持 idle,升级卡片不出现(升级是增强,不是硬依赖)
    onError: (e) => console.warn("[update] 升级状态同步失败,保持 idle", e),
  })

  // 卡片动作:全部走 Rust 命令(检查/下载/安装/重启都在 Rust 侧,见 update.rs)
  const applyUpdate = useCallback(() => {
    void invoke("update_apply").catch(() => {})
  }, [])
  const restartNow = useCallback(() => {
    void invoke("update_restart").catch(() => {})
  }, [])
  const dismiss = useCallback(() => {
    void invoke("update_dismiss").catch(() => {})
  }, [])
  const openReleases = useCallback(() => {
    void openUrl(RELEASES_URL).catch(() => {})
  }, [])

  return {
    status,
    error,
    version,
    currentVersion,
    notes,
    downloadedBytes,
    totalBytes,
    visible: isUpdateCardVisible(status, requested),
    applyUpdate,
    restartNow,
    dismiss,
    openReleases,
  }
}
