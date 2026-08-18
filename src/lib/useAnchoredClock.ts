// boot 全程耗时显示(F5 / #7):事件携带秒数 → 锚点插值 → 每秒显示。
// 原 useBoot 内联四件套(state×2 + anchor ref + interval + 插值 memo)整体
// 搬迁:锚点重置(setBase)、停表条件(running 输入)与挂载补偿(锚点插值——
// 挂载晚于启动也不丢已过时间)内部化。
//
// 同名不同义在 interface 处消除:本 hook 的输出 elapsedSecs 是「插值后的
// 显示秒数」(每秒重算),与事件携带的原始秒数(useBoot 的 setBase 输入)是
// 两个概念,不再共用同一名字。
//
// 时序不变量(纯核心 reduceClock,可测):
// - 重试锚点重置:setBase 无条件覆盖旧锚点,新 boot 从干净起点开始
// - error 停表不残留:running=false 时 stop 清锚点——停表后不残留旧起点,
//   重试新事件到达前不沿用旧秒数继续插值

import { useCallback, useEffect, useMemo, useReducer } from "react"

import { interpolateElapsed } from "@/lib/elapsed"

export interface ClockAnchor {
  /** 最近一次事件携带的累计秒数(Rust 真实流水线起点) */
  baseSecs: number
  /** 该事件到达的本地时刻(ms) */
  atMs: number
}

export interface ClockState {
  /** 插值锚点(null = 未起步 / 已停表) */
  anchor: ClockAnchor | null
  /** 每秒 tick 计数(驱动插值重算;无 setTimeout 漂移) */
  tick: number
}

export type ClockEvent =
  /** 新秒数到达(事件/快照携带):重置锚点(重试/新起点) */
  | { type: "set-base"; baseSecs: number; atMs: number }
  /** 每秒:tick 计数 +1,渲染层按锚点重插值 */
  | { type: "tick" }
  /** 停表(idle/error):清锚点,不残留旧起点 */
  | { type: "stop" }

export function reduceClock(state: ClockState, event: ClockEvent): ClockState {
  switch (event.type) {
    case "set-base":
      // 无条件覆盖:重试后的新事件从干净起点开始,不沿用旧锚点
      return { anchor: { baseSecs: event.baseSecs, atMs: event.atMs }, tick: state.tick }
    case "tick":
      return { ...state, tick: state.tick + 1 }
    case "stop":
      // 停表即清零(锚点 + tick);幂等:已停表不产生新状态,免无谓重渲染
      return state.anchor === null ? state : { anchor: null, tick: 0 }
  }
}

const INITIAL: ClockState = { anchor: null, tick: 0 }

export function useAnchoredClock({ running }: { running: boolean }): {
  /** 插值后的显示秒数(null = 未起步 / 已停表;每秒重算) */
  elapsedSecs: number | null
  /** 事件携带秒数到达:重置锚点(调用方在 applyView 里接线) */
  setBase: (secs: number) => void
} {
  const [state, dispatch] = useReducer(reduceClock, INITIAL)

  // 停表条件(running = phase 不在 idle/error):清锚点并停 tick——不残留
  // 旧起点,重试后新事件 setBase 从干净起点开始
  useEffect(() => {
    if (!running) dispatch({ type: "stop" })
  }, [running])

  // 时钟:有锚点后每秒 tick 一次;停表后锚点已清,无 tick 也无残留
  useEffect(() => {
    if (state.anchor === null) return
    const id = setInterval(() => dispatch({ type: "tick" }), 1000)
    return () => clearInterval(id)
  }, [state.anchor])

  // 渲染值:锚点 + 当前时刻插值(每秒 tick 重渲染;无 setTimeout 漂移,
  // 挂载晚于启动也不丢已过时间)
  const elapsedSecs = useMemo(() => {
    const a = state.anchor
    if (a === null) return null
    return interpolateElapsed(a.baseSecs, a.atMs, Date.now())
  }, [state.anchor, state.tick])

  const setBase = useCallback((secs: number) => {
    dispatch({ type: "set-base", baseSecs: secs, atMs: Date.now() })
  }, [])

  return { elapsedSecs, setBase }
}
