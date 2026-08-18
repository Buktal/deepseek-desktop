// 全屏覆盖层互斥判别的全组合测试(F4 四态):deriveOverlay 直收原始输入
// (status + requested),可见性策略收在 derive 内部——原先 isUpdateCardVisible /
// isUpgradeCardVisible 的独立测试并入本文件,优先级规则(Error > Upgrade >
// Boot > Update)与可见性规则单点由全组合扫描守住。
import { describe, expect, it } from "vitest"

import { deriveOverlay, type OverlayState } from "@/features/shell/derive"

// 全空闲基线:boot 就绪且已揭示、两层升级 idle 未请求、dsh 在跑 → 无覆盖层
const base: OverlayState = {
  bootPhase: "ready",
  bootRevealed: true,
  upgradeStatus: "idle",
  upgradeRequested: false,
  updateStatus: "idle",
  updateRequested: false,
  dshDown: false,
}

function s(partial: Partial<OverlayState>): OverlayState {
  return { ...base, ...partial }
}

describe("deriveOverlay", () => {
  it("四路全空闲 → None(iframe 直显)", () => {
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

  it("dsh 升级覆盖层可见(active/ready/failed 必显)→ Upgrade,盖过 boot", () => {
    for (const status of ["active", "ready", "failed"] as const) {
      expect(deriveOverlay(s({ upgradeStatus: status }))).toEqual({ kind: "upgrade", status })
      // boot 各阶段同屏(防御组合):升级优先
      expect(deriveOverlay(s({ upgradeStatus: status, bootPhase: "checking" }))).toEqual({
        kind: "upgrade",
        status,
      })
    }
  })

  it("dsh 升级 available 需显式请求:未请求不弹,请求后弹", () => {
    expect(deriveOverlay(s({ upgradeStatus: "available" }))).toBeNull()
    expect(deriveOverlay(s({ upgradeStatus: "available", upgradeRequested: true }))).toEqual({
      kind: "upgrade",
      status: "available",
    })
    // idle 即使带请求也不弹(请求只对 available 有意义,生命周期在消费后复位)
    expect(deriveOverlay(s({ upgradeStatus: "idle", upgradeRequested: true }))).toBeNull()
  })

  it("应用升级卡 available 需显式请求,failed 必显", () => {
    expect(deriveOverlay(s({ updateStatus: "available" }))).toBeNull()
    expect(deriveOverlay(s({ updateStatus: "available", updateRequested: true }))).toEqual({
      kind: "update",
      status: "available",
    })
    // failed(失败降级 GitHub 手动下载)必显
    expect(deriveOverlay(s({ updateStatus: "failed" }))).toEqual({ kind: "update", status: "failed" })
  })

  it("应用升级卡 idle/checking/downloading/ready 不渲染卡片", () => {
    // downloading/ready 由右下角非模态浮层 UpdateFloat 呈现,不渲染卡片(#31)
    for (const status of ["idle", "checking", "downloading", "ready"] as const) {
      expect(deriveOverlay(s({ updateStatus: status, updateRequested: true }))).toBeNull()
    }
  })

  it("Update 优先级最低:boot 未揭示 / 升级覆盖层 / 意外退出期间一律让位", () => {
    // boot 各阶段同屏:update 卡让位 boot(原先由 App 表达式求值顺序保证)
    for (const phase of ["idle", "checking", "installing", "starting", "error"] as const) {
      expect(
        deriveOverlay(s({ bootPhase: phase, updateStatus: "available", updateRequested: true })),
      ).toEqual({ kind: "boot", phase })
    }
    // 就绪退出动画未完成:update 卡让位 boot
    expect(
      deriveOverlay(s({ bootRevealed: false, updateStatus: "available", updateRequested: true })),
    ).toEqual({ kind: "boot", phase: "ready" })
    // 升级覆盖层可见:update 卡让位 upgrade
    expect(
      deriveOverlay(
        s({
          upgradeStatus: "failed",
          updateStatus: "available",
          updateRequested: true,
        }),
      ),
    ).toEqual({ kind: "upgrade", status: "failed" })
    // dsh 意外退出:update 卡让位 error
    expect(deriveOverlay(s({ dshDown: true, updateStatus: "available", updateRequested: true }))).toEqual(
      { kind: "error" },
    )
  })

  it("Error 优先:dsh 意外退出盖过 boot 与两张卡", () => {
    expect(deriveOverlay(s({ dshDown: true }))).toEqual({ kind: "error" })
    // 盖过 boot 各阶段
    expect(deriveOverlay(s({ dshDown: true, bootPhase: "checking" }))).toEqual({ kind: "error" })
    expect(deriveOverlay(s({ dshDown: true, bootPhase: "error" }))).toEqual({ kind: "error" })
    // 盖过升级卡(failed 决策面等)——dsh 已死,先恢复服务
    expect(
      deriveOverlay(s({ dshDown: true, upgradeStatus: "failed", upgradeRequested: true })),
    ).toEqual({ kind: "error" })
    expect(
      deriveOverlay(s({ dshDown: true, upgradeStatus: "ready", upgradeRequested: true })),
    ).toEqual({ kind: "error" })
    // 盖过应用升级卡(failed 必显也要让位)
    expect(
      deriveOverlay(s({ dshDown: true, updateStatus: "failed", updateRequested: true })),
    ).toEqual({ kind: "error" })
  })

  it("升级流水线在途抑制误报:杀旧 dsh 是流水线一部分,不报 Error", () => {
    expect(
      deriveOverlay(s({ dshDown: true, upgradeStatus: "active", upgradeRequested: true })),
    ).toEqual({ kind: "upgrade", status: "active" })
  })

  it("互斥:任意组合单一时刻至多一个覆盖层(判别幂等)", () => {
    // 组合扫描:四路状态全组合,结果恒为 null 或互斥四态之一(无多覆盖层可能)
    const phases = ["idle", "checking", "installing", "starting", "ready", "error"] as const
    const upgradeStatuses = ["idle", "available", "active", "ready", "failed"] as const
    const updateStatuses = ["idle", "checking", "available", "downloading", "ready", "failed"] as const
    for (const bootPhase of phases) {
      for (const bootRevealed of [false, true]) {
        for (const upgradeStatus of upgradeStatuses) {
          for (const upgradeRequested of [false, true]) {
            for (const updateStatus of updateStatuses) {
              for (const updateRequested of [false, true]) {
                for (const dshDown of [false, true]) {
                  const derived = deriveOverlay(
                    s({
                      bootPhase,
                      bootRevealed,
                      upgradeStatus,
                      upgradeRequested,
                      updateStatus,
                      updateRequested,
                      dshDown,
                    }),
                  )
                  expect(
                    derived === null ||
                      derived.kind === "boot" ||
                      derived.kind === "upgrade" ||
                      derived.kind === "update" ||
                      derived.kind === "error",
                  ).toBe(true)
                  // 组合测试的期望由优先级规则推导,与上述定向用例保持一致:
                  // Error(非升级在途)> Upgrade > Boot > Update
                  if (dshDown && upgradeStatus !== "active") {
                    expect(derived).toEqual({ kind: "error" })
                  } else if (
                    upgradeStatus === "available" ? upgradeRequested : upgradeStatus !== "idle"
                  ) {
                    expect(derived).toEqual({ kind: "upgrade", status: upgradeStatus })
                  } else if (bootPhase !== "ready" || !bootRevealed) {
                    expect(derived).toEqual({ kind: "boot", phase: bootPhase })
                  } else if (
                    updateStatus === "available" ? updateRequested : updateStatus === "failed"
                  ) {
                    expect(derived).toEqual({ kind: "update", status: updateStatus })
                  } else {
                    expect(derived).toBeNull()
                  }
                }
              }
            }
          }
        }
      }
    }
  })
})
