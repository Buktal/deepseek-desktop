// dsh 升级链的前端接线(#17):镜像 Rust 侧升级状态 + 卡片动作。
//
// 架构与 useUpdateCheck 同款(#9 约束):检查/流水线全在 Rust 侧(upgrade.rs:
// 启动探测 + 6h 轮询 + 托盘手动入口),本 hook 只是本地页上的镜像视图——
// 挂载时拉 `upgrade_state` 快照 + 监听 `upgrade-state` 事件,动作走命令
// (upgrade_confirm / upgrade_dismiss)。
//
// 升级卡镜像基建(F1):requested 生命周期 / card-request 监听走
// useTrayCardMirror 共享实现(useUpdateCheck 同款),本 hook 只声明差异面:
// 字段映射(apply)、快照命令与动作命令表;卡片可见性由 deriveOverlay 内部
// 策略判定(F4),本 hook 暴露原始输入(status + requested)。
//
// 错误跨边界保持结构化形态({kind,data}),卡片渲染时才经 localizeStructuredError
// 翻译(语言切换可重译,见 src/lib/error.ts 与 #12)。

import { invoke } from "@tauri-apps/api/core"
import { useState } from "react"

import { toStructuredError, type RustErrorPayload, type StructuredError } from "@/lib/error"
import type { InstallStage } from "@/lib/installStage"
import { useTrayCardMirror } from "@/lib/useTrayCardMirror"

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
  stage?: InstallStage | null
  error?: RustErrorPayload | null
}

export function useDshUpgrade() {
  // 跨边界保持结构化形态,卡片渲染时才翻译(语言切换不冻结旧文案)
  const [error, setError] = useState<StructuredError | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const [currentVersion, setCurrentVersion] = useState<string | null>(null)
  const [phase, setPhase] = useState<DshUpgradePhase | null>(null)
  const [progress, setProgress] = useState<number | null>(null)
  const [stage, setStage] = useState<InstallStage | null>(null)

  const mirror = useTrayCardMirror({
    stateEvent: "upgrade-state",
    cardRequestEvent: "upgrade-card-request",
    snapshot: () => invoke<DshUpgradeStateView>("upgrade_state"),
    // status/requested 由 mirror 接管,这里只写其余字段(事件与快照同构,
    // 后到者覆盖,无竞态)
    apply: (view) => {
      setError(toStructuredError(view.error ?? null))
      setVersion(view.version ?? null)
      setCurrentVersion(view.currentVersion ?? null)
      setPhase(view.phase ?? null)
      setProgress(view.progress ?? null)
      setStage(view.stage ?? null)
    },
    actions: {
      confirm: "upgrade_confirm",
      dismiss: "upgrade_dismiss",
    },
    // 监听/快照失败:保持 idle,升级卡片不出现(升级是增强,不是硬依赖)
    onError: (e) => console.warn("[upgrade] dsh 升级状态同步失败,保持 idle", e),
  })

  return {
    ...mirror,
    error,
    version,
    currentVersion,
    phase,
    progress,
    stage,
  }
}
