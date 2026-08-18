// useAnchoredClock 的纯核心时序测试(F5):重试锚点重置、error 停表不残留。
// 生产路径:useBoot 每次事件携带秒数时 setBase(重置锚点);phase 离开
// idle/error 时 running 输入停表(interval 与插值都依赖锚点,停表即清零)。
import { describe, expect, it } from "vitest"

import { reduceClock, type ClockEvent, type ClockState } from "@/lib/useAnchoredClock"

const initial: ClockState = { anchor: null, tick: 0 }

function run(initial: ClockState, events: ClockEvent[]) {
  return events.reduce(reduceClock, initial)
}

describe("reduceClock", () => {
  it("set-base 重置锚点(重试/新起点:旧锚点被新事件覆盖)", () => {
    const started = run(initial, [{ type: "set-base", baseSecs: 120, atMs: 1000 }])
    expect(started.anchor).toEqual({ baseSecs: 120, atMs: 1000 })
    const restarted = run(started, [{ type: "set-base", baseSecs: 0, atMs: 5000 }])
    expect(restarted.anchor).toEqual({ baseSecs: 0, atMs: 5000 })
    expect(restarted.tick).toBe(0)
  })

  it("tick 计数递增(每秒重渲染驱动插值,锚点不动)", () => {
    const s = run(initial, [
      { type: "set-base", baseSecs: 10, atMs: 1000 },
      { type: "tick" },
      { type: "tick" },
    ])
    expect(s.tick).toBe(2)
    expect(s.anchor).toEqual({ baseSecs: 10, atMs: 1000 })
  })

  it("stop 清锚点(error/待机停表:锚点不残留,重试后不沿用旧起点)", () => {
    const s = run(initial, [
      { type: "set-base", baseSecs: 120, atMs: 1000 },
      { type: "tick" },
      { type: "stop" },
    ])
    expect(s).toEqual(initial)
  })

  it("停表幂等:已停表收到 stop 不产生新状态(免无谓重渲染)", () => {
    const stopped = reduceClock(initial, { type: "stop" })
    expect(reduceClock(stopped, { type: "stop" })).toBe(stopped)
  })

  it("时序序列:起步 → 停表(error)→ 重试新事件(锚点干净重置)", () => {
    const s = run(initial, [
      { type: "set-base", baseSecs: 120, atMs: 1000 },
      { type: "tick" },
      { type: "stop" },
      { type: "set-base", baseSecs: 3, atMs: 8000 },
    ])
    expect(s.anchor).toEqual({ baseSecs: 3, atMs: 8000 })
    expect(s.tick).toBe(0)
  })

  it("未起步时 stop 无副作用,保持初始状态", () => {
    expect(reduceClock(initial, { type: "stop" })).toBe(initial)
  })
})
