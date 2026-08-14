import { describe, expect, it } from "vitest"

import { reduceBootExit, type BootExitEvent } from "@/lib/bootExit"

// 生产路径:useBootExit 的 reducer 逐事件归约(先 ready → pending → 信号 →
// signaled),动画结束时 onAnimationEnd / 兜底定时器二选一触发。
// 本测试覆盖全部事件 × 状态组合的守卫语义。
function run(initial: "idle" | "pending" | "signaled", events: BootExitEvent[]) {
  return events.reduce(reduceBootExit, initial)
}

describe("reduceBootExit", () => {
  it("ready 进入 pending(开始播放退出动画)", () => {
    expect(run("idle", [{ type: "ready" }])).toBe("pending")
  })

  it("重复 ready 幂等(快照+事件双到达不重启动画)", () => {
    expect(run("idle", [{ type: "ready" }, { type: "ready" }])).toBe("pending")
  })

  it("动画结束 → signaled", () => {
    expect(run("idle", [{ type: "ready" }, { type: "animation-end" }])).toBe("signaled")
  })

  it("兜底定时器到点 → signaled(动画事件缺失时导航不依赖 CSS)", () => {
    expect(run("idle", [{ type: "ready" }, { type: "fallback" }])).toBe("signaled")
  })

  it("signaled 后动画结束事件重复触发不重发信号", () => {
    expect(run("idle", [{ type: "ready" }, { type: "animation-end" }, { type: "animation-end" }])).toBe(
      "signaled",
    )
  })

  it("pending 中 phase 离开 ready → 复位 idle 不信号", () => {
    // 错误/重试:动画取消,不得再发导航信号(Rust 侧超时兜底导航)
    expect(run("idle", [{ type: "ready" }, { type: "left-ready" }])).toBe("idle")
    // 复位后迟到的事件全部忽略
    expect(
      run("idle", [
        { type: "ready" },
        { type: "left-ready" },
        { type: "animation-end" },
        { type: "fallback" },
      ]),
    ).toBe("idle")
  })

  it("signaled 后的 left-ready 复位(下一轮 boot 从干净状态开始)", () => {
    expect(run("idle", [{ type: "ready" }, { type: "animation-end" }, { type: "left-ready" }])).toBe(
      "idle",
    )
  })

  it("signaled 后的新 ready 是下一轮 boot,重新进入 pending", () => {
    expect(
      run("idle", [
        { type: "ready" },
        { type: "animation-end" },
        { type: "ready" },
        { type: "animation-end" },
      ]),
    ).toBe("signaled")
  })

  it("idle 收到动画结束/兜底事件忽略(迟到事件无副作用)", () => {
    expect(run("idle", [{ type: "animation-end" }])).toBe("idle")
    expect(run("idle", [{ type: "fallback" }])).toBe("idle")
  })

  it("idle 收到 left-ready 保持 idle", () => {
    expect(run("idle", [{ type: "left-ready" }])).toBe("idle")
  })
})
