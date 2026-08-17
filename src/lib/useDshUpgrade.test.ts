// dsh 升级卡片可见性纯函数测试:浮层可见性规则(App.tsx 渲染路径,
// 生产路径即 App.tsx / useDshUpgrade.ts 的渲染决策)
import { describe, expect, it } from "vitest"

import {
  isUpgradeCardVisible,
  type DshUpgradeStatus,
} from "@/lib/useDshUpgrade"

describe("isUpgradeCardVisible", () => {
  it("always shows card while pipeline is active-ish", () => {
    for (const s of ["active", "ready", "failed"] as const) {
      expect(isUpgradeCardVisible(s satisfies DshUpgradeStatus, false)).toBe(true)
    }
  })

  it("shows available card only when explicitly requested", () => {
    // 自动检测只亮托盘徽标,不弹卡片(#3 §1);托盘「升级 dsh 到 vX」请求后才弹
    expect(isUpgradeCardVisible("available", false)).toBe(false)
    expect(isUpgradeCardVisible("available", true)).toBe(true)
  })

  it("treats idle as invisible", () => {
    expect(isUpgradeCardVisible("idle", true)).toBe(false)
    expect(isUpgradeCardVisible("idle", false)).toBe(false)
  })
})
