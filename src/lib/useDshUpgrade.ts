// dsh 升级链的前端接线(#17):镜像 Rust 侧升级状态 + 卡片动作。
//
// 架构与 useUpdateCheck 同款(#9 约束):检查/流水线全在 Rust 侧(upgrade.rs:
// 启动探测 + 6h 轮询 + 托盘手动入口),本 hook 只是本地页上的镜像视图——
// 挂载时拉 `upgrade_state` 快照 + 监听 `upgrade-state` 事件(同步骨架走
// useRustStateSync,先注册监听再拉快照,后到者覆盖),动作走命令
// (upgrade_confirm / upgrade_dismiss)。StrictMode 双挂载无害:监听注册/清理
// 成对,快照拉取幂等。
//
// 错误跨边界保持结构化形态({kind,data}),卡片渲染时才经 localizeStructuredError
// 翻译(语言切换可重译,见 src/lib/error.ts 与 #12)。

import { invoke } from "@tauri-apps/api/core"
import { useCallback, useState } from "react"

import { toStructuredError, type RustErrorPayload, type StructuredError } from "@/lib/error"
import { useRustStateSync } from "@/lib/useRustStateSync"

export type DshUpgradeStatus =
  | "idle" // 无待升级(初始 / 无新版 / 升级成功消费完毕)
  | "available" // 发现新版,等待用户确认
  | "active" // 升级流水线运行中(phase 区分阶段)
  | "ready" // 升级成功(瞬态:Rust 随即导航回 dsh 页)
  | "failed" // 升级失败(旧版保留;可重试 / 返回 dsh)

export type DshUpgradePhase = "killing" | "installing" | "verifying" | "starting"

/** Rust 侧 UpgradeStateView 的序列化形态(内部 tag status,字段 camelCase) */
export interface DshUpgradeStateView {
  status: DshUpgradeStatus
  version?: string | null
  currentVersion?: string | null
  phase?: DshUpgradePhase | null
  progress?: number | null
  stage?: string | null
  error?: RustErrorPayload | null
}

/**
 * 升级状态是否为「需要展示升级卡片」的活跃态。
 * App 挂载时用它决定先渲染 UpgradeScreen 还是走 boot 分发(#3 §5)。
 * 纯函数,可测。
 */
export function isActiveDshUpgradeStatus(status: DshUpgradeStatus | undefined): boolean {
  return status === "available" || status === "active" || status === "ready" || status === "failed"
}

export function useDshUpgrade() {
  const [status, setStatus] = useState<DshUpgradeStatus>("idle")
  // 跨边界保持结构化形态,卡片渲染时才翻译(语言切换不冻结旧文案)
  const [error, setError] = useState<StructuredError | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const [currentVersion, setCurrentVersion] = useState<string | null>(null)
  const [phase, setPhase] = useState<DshUpgradePhase | null>(null)
  const [progress, setProgress] = useState<number | null>(null)
  const [stage, setStage] = useState<string | null>(null)

  // 事件与快照来自同一 Rust 状态,后到者覆盖,无竞态
  const applyView = useCallback((view: DshUpgradeStateView) => {
    setStatus(view.status)
    setError(toStructuredError(view.error ?? null))
    setVersion(view.version ?? null)
    setCurrentVersion(view.currentVersion ?? null)
    setPhase(view.phase ?? null)
    setProgress(view.progress ?? null)
    setStage(view.stage ?? null)
  }, [])

  useRustStateSync({
    event: "upgrade-state",
    snapshot: () => invoke<DshUpgradeStateView>("upgrade_state"),
    apply: applyView,
    // 监听/快照失败:保持 idle,升级卡片不出现(升级是增强,不是硬依赖)
    onError: (e) => console.warn("[upgrade] dsh 升级状态同步失败,保持 idle", e),
  })

  // 卡片动作:全部走 Rust 命令(检测/流水线/恢复服务都在 Rust 侧,见 upgrade.rs)
  const confirm = useCallback(() => {
    void invoke("upgrade_confirm").catch(() => {})
  }, [])
  const dismiss = useCallback(() => {
    void invoke("upgrade_dismiss").catch(() => {})
  }, [])

  return {
    status,
    error,
    version,
    currentVersion,
    phase,
    progress,
    stage,
    confirm,
    dismiss,
  }
}
