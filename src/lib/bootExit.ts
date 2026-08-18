// boot 就绪 → boot 浮层退出的动画编排(壳页常驻后无窗口导航,#36):
// 动画只做视觉过渡,绝不阻塞 dsh 呈现。
//
// 时序:phase=ready 且 dsh URL 已推给壳页 → 播放退出动画(容器溶解 + 圆环
// 收缩)→ 动画结束(onAnimationEnd)或兜底定时器到点 → signaled,浮层卸载。
//
// 状态机为纯函数(可测):idle →(ready)→ pending →(animation-end | fallback)
// → signaled。守卫:重复 ready 幂等(快照+事件双到达不重启动画)、signaled 后
// 不再变更(动画结束事件可能重复触发)、phase 离开 ready(错误/重试)复位。
//
// 动画契约单一出口(Q1):时长与动画名全部从这里导出——BootScreen 把时长写
// 进 --boot-exit-ms 变量(index.css 消费)并用于 onAnimationEnd 过滤,兜底
// 定时器时长由此推导;index.css 不再硬编码时长。兜底定时器只保证「退出
// 不依赖 CSS 事件」,动画本身在 CSS 侧完成,这里不复制动画逻辑。

/** 退出动画时长(ms)。单一事实来源:index.css 经 --boot-exit-ms 变量消费,
 * BootScreen 从本常量写入该变量;BOOT_EXIT_FALLBACK_MS 由此推导 */
export const BOOT_EXIT_ANIMATION_MS = 400
/** 退出动画名(单一事实来源:BootScreen 的 onAnimationEnd 过滤用) */
export const BOOT_EXIT_ANIMATION_NAME = "boot-exit"
/** 旋转圆环收缩动画名(单一事实来源:同上) */
export const BOOT_EXIT_RING_ANIMATION_NAME = "boot-exit-ring"
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
