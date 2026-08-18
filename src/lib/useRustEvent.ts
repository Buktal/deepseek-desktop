// 一次性事件监听样板收敛(F3):「alive 标志 + listen().then(un) + 双清理 +
// StrictMode 挂卸时序」的同一模式,原先手写五份(useBoot 的 dsh-url /
// dsh-exited、useDshUpgrade / useUpdateCheck 的 card-request、ShellDialogs
// 的 shell-dialog),每份 ~15 行;现收敛为单一实现(先例:rustStateSync.ts
// 归并四份手写 effect)。
//
// 语义(与旧手写样板逐字等价):
// - 挂载注册监听,卸载反注册;StrictMode 双挂载无害(注册/清理成对)
// - 清理先于异步解析完成:解析完成后立即反注册,不残留监听
// - guard(可选):载荷守卫,校验不通过不调 handler(如 shell-dialog 的
//   isShellDialogRequest);无 guard 时按契约断言载荷形状,提取防御由
//   handler 自行负责
// - 配置走 ref:effect 只跑一次,inline 闭包不引起监听重挂

import { listen } from "@tauri-apps/api/event"
import { useEffect, useRef } from "react"

export function useRustEvent<T = unknown>(
  /** 事件名(Rust emit 方唯一事实源) */
  event: string,
  /** 载荷处理(事件到达时调用;alive 守卫保证清理后不再调用) */
  handler: (payload: T) => void,
  /** 载荷守卫(可选):校验不通过静默丢弃,不调 handler */
  guard?: (payload: unknown) => payload is T,
): void {
  const handlerRef = useRef({ handler, guard })
  handlerRef.current = { handler, guard }

  useEffect(() => {
    let alive = true
    let stop: (() => void) | undefined
    void listen(event, (e) => {
      if (!alive) return
      const payload: unknown = e.payload
      const { handler, guard } = handlerRef.current
      if (guard && !guard(payload)) return
      handler(payload as T)
    }).then((un) => {
      stop = un
      if (!alive) un() // 清理先于异步解析完成:立即停,不残留监听
    })
    return () => {
      alive = false
      stop?.()
    }
  }, [event])
}
