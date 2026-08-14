import { describe, expect, it } from "vitest"

import { formatElapsed, interpolateElapsed } from "@/lib/elapsed"

describe("formatElapsed", () => {
  it("formats seconds below one minute (issue spec: 1 秒)", () => {
    expect(formatElapsed(0)).toEqual({ minutes: 0, seconds: 0 })
    expect(formatElapsed(1)).toEqual({ minutes: 0, seconds: 1 })
    expect(formatElapsed(59)).toEqual({ minutes: 0, seconds: 59 })
  })

  it("formats minutes and seconds (issue spec: N 分 M 秒)", () => {
    expect(formatElapsed(60)).toEqual({ minutes: 1, seconds: 0 })
    expect(formatElapsed(83)).toEqual({ minutes: 1, seconds: 23 })
    expect(formatElapsed(125)).toEqual({ minutes: 2, seconds: 5 })
    expect(formatElapsed(3600)).toEqual({ minutes: 60, seconds: 0 })
  })

  it("defends against non-finite or negative input", () => {
    expect(formatElapsed(-5)).toEqual({ minutes: 0, seconds: 0 })
    expect(formatElapsed(Number.NaN)).toEqual({ minutes: 0, seconds: 0 })
    expect(formatElapsed(Number.POSITIVE_INFINITY)).toEqual({ minutes: 0, seconds: 0 })
    expect(formatElapsed(1.9)).toEqual({ minutes: 0, seconds: 1 }) // 取整
  })
})

describe("interpolateElapsed", () => {
  // 生产路径:useBoot 的 displayElapsedSecs 按锚点插值(挂载晚于启动也不丢已过时间)
  it("按锚点插值:每秒 tick 显示递增不漂移", () => {
    expect(interpolateElapsed(10, 1000, 1000)).toBe(10)
    expect(interpolateElapsed(10, 1000, 1999)).toBe(10) // 不足一秒不跳
    expect(interpolateElapsed(10, 1000, 2000)).toBe(11)
    expect(interpolateElapsed(10, 1000, 3000)).toBe(12)
  })

  it("非整数差值向下取整(与 formatElapsed 同语义)", () => {
    expect(interpolateElapsed(0, 0, 2500)).toBe(2)
    expect(interpolateElapsed(5, 100, 234)).toBe(5)
  })
})
