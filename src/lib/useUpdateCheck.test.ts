// 升级状态纯函数测试:App 路由守卫与下载百分比(App 挂载 → 升级卡片的分发条件,
// 生产路径即 App.tsx / UpdateCard.tsx 的渲染路径)
import { describe, expect, it } from "vitest"

import {
  isActiveUpdateStatus,
  updatePercent,
  type UpdateStatus,
} from "@/lib/useUpdateCheck"

describe("isActiveUpdateStatus", () => {
  it("treats the four card states as active", () => {
    for (const s of ["available", "downloading", "ready", "failed"] as const) {
      expect(isActiveUpdateStatus(s satisfies UpdateStatus)).toBe(true)
    }
  })

  it("treats idle/checking/undefined as inactive", () => {
    expect(isActiveUpdateStatus("idle")).toBe(false)
    expect(isActiveUpdateStatus("checking")).toBe(false)
    expect(isActiveUpdateStatus(undefined)).toBe(false)
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
