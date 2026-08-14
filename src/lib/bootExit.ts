// boot 就绪 → 导航的退出动画编排(#4):动画只做视觉过渡,绝不阻塞导航。
//
// 时序:phase=ready → 播放退出动画(容器溶解 + 圆环收缩)→ 动画结束
// (onAnimationEnd)或兜底定时器到点 → 信号 navigate_to_dsh → Rust 侧
// boot_pipeline 收到信号后照常导航(1.5s 超时兜底:前端故障不阻塞 boot)。
//
// 状态机为纯函数(可测):idle →(ready)→ pending →(animation-end | fallback)
// → signaled。守卫:重复 ready 幂等(快照+事件双到达不重启动画)、signaled 后
// 不再信号(动画结束事件可能重复触发,信号只发一次)、phase 离开 ready
// (错误/重试)复位不信号。
//
// 动画时长常量与 index.css 的 boot-exit / boot-exit-ring keyframes 时长一致
// (两处各持一份,改动时同步);兜底定时器只保证「导航不依赖 CSS 事件」,
// 动画本身在 CSS 侧完成,这里不复制动画逻辑。

/** 退出动画时长(ms)。与 index.css 的 boot-exit / boot-exit-ring 时长一致 */
export const BOOT_EXIT_ANIMATION_MS = 400
/** 兜底信号定时器时长:动画时长 + 余量。CSS 动画事件在 reduced-motion 下
 * 不触发(animation: none),定时器保证导航不依赖 CSS 事件 */
export const BOOT_EXIT_FALLBACK_MS = BOOT_EXIT_ANIMATION_MS + 100

export type BootExitState = "idle" | "pending" | "signaled"

export type BootExitEvent =
  | { type: "ready" } // phase 变为 ready:开始退出动画
  | { type: "left-ready" } // phase 离开 ready(错误/重试):取消退出,不信号
  | { type: "animation-end" } // 退出动画结束(CSS onAnimationEnd)
  | { type: "fallback" } // 兜底定时器到点(动画事件缺失时)

export function reduceBootExit(state: BootExitState, event: BootExitEvent): BootExitState {
  switch (event.type) {
    case "ready":
      // 幂等:重复 ready(快照+事件双到达)不重启动画;signaled 后的新 ready
      // 是下一轮 boot,重新进入 pending
      return "pending"
    case "left-ready":
      return "idle"
    case "animation-end":
    case "fallback":
      // 一旦 signaled 不再变更:动画结束事件可能重复触发,信号只发一次
      return state === "pending" ? "signaled" : state
  }
}
