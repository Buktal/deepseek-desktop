// boot 退出动画的时序接线(#4):把纯状态机(reduceBootExit)接到 React 生命周期。
// 外部资源(invoke / setTimeout)全部在 effect 内获取(node 环境可 import,
// 状态机逻辑本身是纯函数,在 bootExit.ts 独立测试)。
import { invoke } from "@tauri-apps/api/core"
import { useCallback, useEffect, useReducer, useRef } from "react"

import { BOOT_EXIT_FALLBACK_MS, reduceBootExit } from "@/lib/bootExit"

/**
 * ready = phase 变为 ready。两种路径都播放退出动画:
 * - 活体 boot 的 ready 事件:Rust 即将导航(等信号),动画在此间播放
 * - 挂载快照即 ready(webview 挂载晚于 boot 完成,实机常态):Rust 已过
 *   等待期,信号命令侧在无等待者时直接导航(Rust 侧 request_navigate 兜底)
 */
export function useBootExit({ ready }: { ready: boolean }) {
  const [state, dispatch] = useReducer(reduceBootExit, "idle")
  // signaled → invoke 只发一次(StrictMode 双 effect + 动画事件重复触发兜底)
  const signalFired = useRef(false)

  useEffect(() => {
    dispatch(ready ? { type: "ready" } : { type: "left-ready" })
  }, [ready])

  // 兜底定时器:动画事件缺失(reduced-motion 下 animation: none 不触发
  // onAnimationEnd)时由定时器到点发信号——导航不依赖 CSS 事件;离开 pending
  // (动画结束或 left-ready)即清除,不在错误/重试后残留
  useEffect(() => {
    if (state !== "pending") return
    const id = window.setTimeout(
      () => dispatch({ type: "fallback" }),
      BOOT_EXIT_FALLBACK_MS,
    )
    return () => window.clearTimeout(id)
  }, [state])

  useEffect(() => {
    if (state !== "signaled" || signalFired.current) return
    signalFired.current = true
    // 信号幂等:Rust 侧 phase 守卫 + 通道生命周期兜底,失败静默(超时兜底导航)
    void invoke("navigate_to_dsh").catch(() => {})
  }, [state])

  // CSS 动画结束事件(经 BootScreen 的 root onAnimationEnd 上报)
  const onExitAnimationEnd = useCallback(() => dispatch({ type: "animation-end" }), [])

  return { exiting: state === "pending", onExitAnimationEnd }
}
