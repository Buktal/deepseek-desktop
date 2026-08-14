import { describe, expect, it } from "vitest"

import { formatElapsed } from "@/lib/elapsed"

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
