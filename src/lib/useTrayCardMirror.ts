// 升级卡片镜像双胞胎的共享基建(F1):useUpdateCheck(应用自身升级)与
// useDshUpgrade(dsh 升级)原先逐段同构——status/requested 状态、card-request
// 监听样板、状态转移复位规则,两份各持一份,分叉迟早漂移;且 requested
// 生命周期(事件置位 / 状态转移复位 / 跨轮不残留)零测试。
// 现收敛为单一实现:生命周期收进纯函数 reduceRequested(可测),hook 只做
// 接线(useRustStateSync 骨架 + card-request 监听 + 动作命令表)。
//
// 语义(与旧实现逐字等价,两层升级的状态机语义不漂移):
// - card-request 事件(托盘「升级到 vX」菜单,tray.rs 推送)→ requested = true
// - 状态视图(status ≠ "available")→ requested = false:请求只对 available
//   有意义,进入流水线/消费完毕(idle)后复位,旧请求不跨轮残留
// - 卡片可见性由 deriveOverlay 内部策略判定(F4),本 hook 只暴露原始输入
//   (status + requested),不再预推导 visible
// - 同步骨架走 useRustStateSync(先注册监听再拉快照,后到者覆盖);
//   动作命令表 → invoke 包装(Rust 动作表是命令名唯一事实源)
//
// 初始 status 为 "idle"(两层流水线均以 idle 起步);"idle" 在调用方的
// status 联合内,泛型定义处以联合形式表达,调用方实例化时归一。

import { invoke } from "@tauri-apps/api/core"
import { useCallback, useReducer, useRef, useState } from "react"

import type { SyncErrorSource } from "@/lib/rustStateSync"
import { useRustEvent } from "@/lib/useRustEvent"
import { useRustStateSync } from "@/lib/useRustStateSync"

/** requested 生命周期事件(纯核心,可测) */
export type RequestedEvent =
  | { type: "card-request" } // 托盘「升级到 vX」菜单点击:置位
  | { type: "state-view"; status: string } // 状态视图(事件或快照)应用

/**
 * requested 生命周期(纯函数):事件置位;状态离开 available 复位——
 * 请求只对 available 有意义,进入流水线/消费完毕(idle)后复位,
 * 旧请求不跨轮残留(下一轮自动检测命中时不会误弹卡片)。
 */
export function reduceRequested(requested: boolean, event: RequestedEvent): boolean {
  switch (event.type) {
    case "card-request":
      return true
    case "state-view":
      return event.status === "available" ? requested : false
  }
}

export interface TrayCardMirrorConfig<
  V extends { status: string },
  A extends Record<string, string>,
> {
  /** 状态事件名(useRustStateSync 骨架:先注册监听,再拉快照,后到者覆盖) */
  stateEvent: string
  /** 托盘「升级到 vX」菜单请求事件名(置位 requested;事件由 tray.rs 推送) */
  cardRequestEvent: string
  /** 快照命令(挂载拉取;与事件同源,后到者覆盖) */
  snapshot: () => Promise<V>
  /** 状态应用入口:视图字段写入调用方 state(status/requested 由本 hook 接管) */
  apply: (view: V) => void
  /** 动作命令表:动作名 → Rust 命令名(hook 生成 invoke 包装,失败静默) */
  actions: A
  /** 监听/快照失败的处理(默认静默;升级是增强,不是硬依赖) */
  onError?: (e: unknown, source: SyncErrorSource) => void
}

export type TrayCardMirrorResult<
  V extends { status: string },
  A extends Record<string, string>,
> = {
  /** 最近一次状态视图的 status(初始 "idle":两层流水线均以 idle 起步) */
  status: V["status"] | "idle"
  /** 托盘显式请求标志(deriveOverlay 的原始输入之一) */
  requested: boolean
} & { [K in keyof A]: () => void }

export function useTrayCardMirror<V extends { status: string }, A extends Record<string, string>>(
  config: TrayCardMirrorConfig<V, A>,
): TrayCardMirrorResult<V, A> {
  const configRef = useRef(config)
  configRef.current = config

  const [status, setStatus] = useState<V["status"] | "idle">("idle")
  const [requested, dispatchRequested] = useReducer(reduceRequested, false)

  // 托盘「升级到 vX」菜单 → 显示升级卡片(事件由 tray.rs 推送;信号事件,
  // 载荷无意义,监听样板走 useRustEvent)
  useRustEvent(config.cardRequestEvent, () => dispatchRequested({ type: "card-request" }))

  // 事件与快照来自同一 Rust 状态,后到者覆盖,无竞态;status/requested 由
  // 本 hook 接管,调用方 apply 只写其余字段(字段映射是各自唯一的差异面)
  const applyView = useCallback((view: V) => {
    setStatus(view.status)
    dispatchRequested({ type: "state-view", status: view.status })
    configRef.current.apply(view)
  }, [])

  useRustStateSync({
    event: config.stateEvent,
    snapshot: config.snapshot,
    apply: applyView,
    onError: config.onError,
  })

  // 卡片动作:命令名 → invoke 包装(命令名是 Rust 动作表的唯一事实源,
  // 调用方只声明「动作名 → 命令名」的映射,失败静默)
  const actions = {} as { [K in keyof A]: () => void }
  for (const name of Object.keys(config.actions) as Array<keyof A>) {
    actions[name] = () => {
      void invoke(config.actions[name]).catch(() => {})
    }
  }

  return {
    status,
    requested,
    ...actions,
  }
}
