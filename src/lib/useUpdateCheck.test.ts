// 升级卡片可见性与下载百分比纯函数测试:浮层可见性规则(App.tsx 渲染路径,
// 生产路径即 App.tsx / useUpdateCheck.ts 的渲染决策)
import { describe, expect, it } from "vitest"

import {
  isUpdateCardVisible,
  updatePercent,
  type UpdateStatus,
} from "@/lib/useUpdateCheck"

describe("isUpdateCardVisible", () => {
  it("always shows card while pipeline is active-ish", () => {
    for (const s of ["downloading", "ready", "failed"] as const) {
      expect(isUpdateCardVisible(s satisfies UpdateStatus, false)).toBe(true)
    }
  })

  it("shows available card only when explicitly requested", () => {
    // 自动检测只亮托盘徽标,不弹卡片(#3 §1);托盘「升级到 vX」请求后才弹
    expect(isUpdateCardVisible("available", false)).toBe(false)
    expect(isUpdateCardVisible("available", true)).toBe(true)
  })

  it("treats idle/checking as invisible", () => {
    expect(isUpdateCardVisible("idle", true)).toBe(false)
    expect(isUpdateCardVisible("checking", true)).toBe(false)
  })
})

describe("updatePercent", () => {
  it("clamps to 100 and rounds", () => {
    expect(updatePercent(50, 100)).toBe(50)
    expect(updatePercent(1, 3)).toBe(33)
    expect(updatePercent(3, 3)).toBe(100)
    expect(updatePercent(120, 100)).toBe(100)
  })

  it("returns null when total is unknown (no content-length)", () => {
    expect(updatePercent(0, 0)).toBeNull()
    expect(updatePercent(42, 0)).toBeNull()
    expect(updatePercent(0, -1)).toBeNull()
  })

  it("returns null on negative downloaded bytes", () => {
    expect(updatePercent(-1, 100)).toBeNull()
  })
})
