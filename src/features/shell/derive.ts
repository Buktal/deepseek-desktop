// 全屏覆盖层互斥编排(#32 拍板 / #40 施工):boot / dsh 升级 / dsh 意外退出
// 三路状态镜像 → 互斥覆盖层,单一时刻至多一个。状态机在纯函数里可测,
// React 只是渲染投影(deriveOverlay 的调用方是 App,渲染由各覆盖层组件完成)。
//
// 优先级:Error > Upgrade > Boot。
// - Error:dsh 意外退出(reaper 的 dsh-exited 事件置位 dshDown)。升级流水线
//   在途时 dsh 死亡是流水线的一部分(killing 杀旧进程),不误报——以
//   upgradeStatus === "active" 为在途标志(与 reaper 现状的非升级判定一致,
//   #32)。Rust 侧 reaper 已按同一语义抑制事件,这里是前端镜像的第二层防线
//   (两路事件通道异步到达,防 dsh-exited 先于 upgrade-state 到达的瞬时误报)。
// - Upgrade:升级覆盖层可见即覆盖(active/ready/failed 必显;available 需
//   托盘显式请求,复用 isUpgradeCardVisible 的纯函数判定)。
// - Boot:boot 流水线各阶段(含 error = boot 失败页 / Node 引导页);ready 后
//   由退出动画撑住覆盖层(bootRevealed = 动画完成、iframe 已揭示),揭示后
//   覆盖层卸载。

import type { Phase } from "@/lib/useBoot"
import type { DshUpgradeStatus } from "@/lib/useDshUpgrade"

/** 覆盖层判别(互斥,单一时刻至多一个;内容由 App 从各自状态流渲染)。 */
export type Overlay =
  | { kind: "boot"; phase: Phase }
  | { kind: "upgrade"; status: DshUpgradeStatus }
  | { kind: "error" }

/** deriveOverlay 的输入:boot / upgrade / dsh 存活三路状态镜像。 */
export interface OverlayState {
  /** boot 流水线阶段("error" = boot 失败,渲染 ErrorScreen / Node 引导页) */
  bootPhase: Phase
  /** boot 就绪揭示动画是否完成(useBootExit 的 done;ready 期间覆盖层由
   *  动画撑住,揭示后卸载——动画是纯装饰,不阻塞 iframe 呈现) */
  bootRevealed: boolean
  /** dsh 升级状态镜像(useDshUpgrade 镜像 Rust upgrade-state) */
  upgradeStatus: DshUpgradeStatus
  /** 升级覆盖层可见(active/ready/failed 必显;available 需显式请求) */
  upgradeVisible: boolean
  /** dsh 意外退出(reaper dsh-exited 事件置位;新 boot 开始 / 新 URL 推达清除) */
  dshDown: boolean
}

export function deriveOverlay(state: OverlayState): Overlay | null {
  const upgradeInFlight = state.upgradeStatus === "active"
  // Error 优先:dsh 意外退出盖过一切(升级流水线在途的杀进程除外,见上)
  if (state.dshDown && !upgradeInFlight) {
    return { kind: "error" }
  }
  // Upgrade:可见即覆盖(升级流水线在途时 dsh 已杀,覆盖层即主画面)
  if (state.upgradeVisible) {
    return { kind: "upgrade", status: state.upgradeStatus }
  }
  // Boot:流水线各阶段 + 就绪退出动画未完成(揭示后卸载)
  if (state.bootPhase !== "ready" || !state.bootRevealed) {
    return { kind: "boot", phase: state.bootPhase }
  }
  return null
}
