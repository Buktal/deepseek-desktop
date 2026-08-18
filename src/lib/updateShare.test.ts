// 下载进度百分比纯函数测试:UpdateFloat 的渲染输入(生产路径即
// components/update/UpdateFloat.tsx 的下载百分比显示)。
import { describe, expect, it } from "vitest"

import { updatePercent } from "@/lib/updateShare"

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
