// boot 就绪 → boot 浮层退出的过渡动画编排(壳页常驻后无窗口导航,#36):
// 时序:phase=ready 且 dsh URL 已推给壳页(iframe 开始加载)→ 播放退出动画
// (容器溶解 + 圆环收缩)→ 动画结束(onAnimationEnd)或兜底定时器到点 →
// done,boot 浮层卸载,reveal iframe。
// 状态机为纯函数(可测):idle →(ready)→ pending →(animation-end | fallback)
// → signaled。守卫:重复 ready 幂等(快照+事件双到达不重启动画)、signaled 后
// 不再变更、phase 离开 ready(错误/重试)复位。
//
// 动画时长常量与 index.css 的 boot-exit / boot-exit-ring keyframes 时长一致
// (两处各持一份,改动时同步);兜底定时器只保证「退出不依赖 CSS 事件」,
// 动画本身在 CSS 侧完成,这里不复制动画逻辑。
import { useCallback, useEffect, useReducer } from "react"

import { BOOT_EXIT_FALLBACK_MS, reduceBootExit } from "@/lib/bootExit"

/**
 * ready = phase 变为 ready 且 dsh URL 已推给壳页。动画是纯装饰:URL 到达即
 * 开始溶解(与 iframe 加载并行),done 后浮层卸载;iframe 加载/CSS 事件
 * 缺失均由兜底定时器覆盖,不阻塞 dsh 呈现。
 */
export function useBootExit({ ready }: { ready: boolean }) {
  const [state, dispatch] = useReducer(reduceBootExit, "idle")

  useEffect(() => {
    dispatch(ready ? { type: "ready" } : { type: "left-ready" })
  }, [ready])

  // 兜底定时器:动画事件缺失(reduced-motion 下 animation: none 不触发
  // onAnimationEnd)时由定时器到点收尾——退出不依赖 CSS 事件;离开 pending
  // (动画结束或 left-ready)即清除,不在错误/重试后残留
  useEffect(() => {
    if (state !== "pending") return
    const id = window.setTimeout(
      () => dispatch({ type: "fallback" }),
      BOOT_EXIT_FALLBACK_MS,
    )
    return () => window.clearTimeout(id)
  }, [state])

  // CSS 动画结束事件(经 BootScreen 的 root onAnimationEnd 上报)
  const onExitAnimationEnd = useCallback(() => dispatch({ type: "animation-end" }), [])

  return { exiting: state === "pending", done: state === "signaled", onExitAnimationEnd }
}
