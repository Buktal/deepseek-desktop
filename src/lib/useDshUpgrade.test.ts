// dsh 升级状态纯函数测试:App 路由守卫(App 挂载 → UpgradeScreen 的分发条件,
// 生产路径即 App.tsx / UpgradeScreen.tsx 的渲染路径)
import { describe, expect, it } from "vitest"

import {
  isActiveDshUpgradeStatus,
  type DshUpgradeStatus,
} from "@/lib/useDshUpgrade"

describe("isActiveDshUpgradeStatus", () => {
  it("treats the four card states as active", () => {
    for (const s of ["available", "active", "ready", "failed"] as const) {
      expect(isActiveDshUpgradeStatus(s satisfies DshUpgradeStatus)).toBe(true)
    }
  })

  it("treats idle/undefined as inactive", () => {
    expect(isActiveDshUpgradeStatus("idle")).toBe(false)
    expect(isActiveDshUpgradeStatus(undefined)).toBe(false)
  })
})
