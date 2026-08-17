import { describe, expect, it } from "vitest"

import { deriveOverlay, type OverlayState } from "@/features/shell/derive"

// 全空闲基线:boot 就绪且已揭示、升级 idle、dsh 在跑 → 无覆盖层
const base: OverlayState = {
  bootPhase: "ready",
  bootRevealed: true,
  upgradeStatus: "idle",
  upgradeVisible: false,
  dshDown: false,
}

function s(partial: Partial<OverlayState>): OverlayState {
  return { ...base, ...partial }
}

describe("deriveOverlay", () => {
  it("三路全空闲 → None(iframe 直显)", () => {
    expect(deriveOverlay(base)).toBeNull()
  })

  it("boot 各阶段 → Boot(含 error:boot 失败页 / Node 引导页)", () => {
    for (const phase of ["idle", "checking", "installing", "starting", "error"] as const) {
      expect(deriveOverlay(s({ bootPhase: phase }))).toEqual({ kind: "boot", phase })
    }
  })

  it("ready 但退出动画未完成 → Boot(动画撑住覆盖层)", () => {
    expect(deriveOverlay(s({ bootPhase: "ready", bootRevealed: false }))).toEqual({
      kind: "boot",
      phase: "ready",
    })
  })

  it("ready 且动画完成 → None", () => {
    expect(deriveOverlay(s({ bootPhase: "ready", bootRevealed: true }))).toBeNull()
  })

  it("升级覆盖层可见(active/ready/failed 必显)→ Upgrade,盖过 boot", () => {
    for (const status of ["active", "ready", "failed"] as const) {
      expect(deriveOverlay(s({ upgradeStatus: status, upgradeVisible: true }))).toEqual({
        kind: "upgrade",
        status,
      })
      // boot 各阶段同屏(防御组合):升级优先
      expect(deriveOverlay(s({ upgradeStatus: status, upgradeVisible: true, bootPhase: "checking" }))).toEqual({
        kind: "upgrade",
        status,
      })
    }
  })

  it("available 需显式请求:未请求不弹,请求后弹", () => {
    expect(deriveOverlay(s({ upgradeStatus: "available", upgradeVisible: false }))).toBeNull()
    expect(deriveOverlay(s({ upgradeStatus: "available", upgradeVisible: true }))).toEqual({
      kind: "upgrade",
      status: "available",
    })
  })

  it("Error 优先:dsh 意外退出盖过 boot 与升级卡", () => {
    expect(deriveOverlay(s({ dshDown: true }))).toEqual({ kind: "error" })
    // 盖过 boot 各阶段
    expect(deriveOverlay(s({ dshDown: true, bootPhase: "checking" }))).toEqual({ kind: "error" })
    expect(deriveOverlay(s({ dshDown: true, bootPhase: "error" }))).toEqual({ kind: "error" })
    // 盖过升级卡(failed 决策面等)——dsh 已死,先恢复服务
    expect(deriveOverlay(s({ dshDown: true, upgradeStatus: "failed", upgradeVisible: true }))).toEqual({
      kind: "error",
    })
    expect(deriveOverlay(s({ dshDown: true, upgradeStatus: "ready", upgradeVisible: true }))).toEqual({
      kind: "error",
    })
  })

  it("升级流水线在途抑制误报:杀旧 dsh 是流水线一部分,不报 Error", () => {
    expect(
      deriveOverlay(s({ dshDown: true, upgradeStatus: "active", upgradeVisible: true })),
    ).toEqual({ kind: "upgrade", status: "active" })
  })

  it("互斥:任意组合单一时刻至多一个覆盖层(判别幂等)", () => {
    // 组合扫描:三路状态全组合,结果恒为 null 或互斥三态之一(无多覆盖层可能)
    const phases = ["idle", "checking", "installing", "starting", "ready", "error"] as const
    const statuses = ["idle", "available", "active", "ready", "failed"] as const
    for (const bootPhase of phases) {
      for (const bootRevealed of [false, true]) {
        for (const upgradeStatus of statuses) {
          for (const upgradeVisible of [false, true]) {
            for (const dshDown of [false, true]) {
              const derived = deriveOverlay(s({ bootPhase, bootRevealed, upgradeStatus, upgradeVisible, dshDown }))
              expect(derived === null || derived.kind === "boot" || derived.kind === "upgrade" || derived.kind === "error").toBe(true)
              // 组合测试的期望由优先级规则推导,与上述定向用例保持一致:
              // Error(非升级在途)> Upgrade > Boot
              if (dshDown && upgradeStatus !== "active") {
                expect(derived).toEqual({ kind: "error" })
              } else if (upgradeVisible) {
                expect(derived).toEqual({ kind: "upgrade", status: upgradeStatus })
              } else if (bootPhase !== "ready" || !bootRevealed) {
                expect(derived).toEqual({ kind: "boot", phase: bootPhase })
              } else {
                expect(derived).toBeNull()
              }
            }
          }
        }
      }
    }
  })
})
