// requested 生命周期(useTrayCardMirror 的纯核心,原 useUpdateCheck /
// useDshUpgrade 各持一份且零测试):事件置位、状态转移复位、跨轮不残留。
// 生产路径:托盘「升级到 vX」菜单(card-request)→ 状态视图(state-view)
// 两条事件流,reduceRequested 是 useTrayCardMirror 接线里的唯一实现。
import { describe, expect, it } from "vitest"

import { reduceRequested, type RequestedEvent } from "@/lib/useTrayCardMirror"

function run(initial: boolean, events: RequestedEvent[]) {
  return events.reduce(reduceRequested, initial)
}

describe("reduceRequested", () => {
  it("card-request 事件置位:任何起点 → true", () => {
    expect(run(false, [{ type: "card-request" }])).toBe(true)
    expect(run(true, [{ type: "card-request" }])).toBe(true)
  })

  it("available 状态视图保留请求(自动检测命中不弹卡,托盘请求仍生效)", () => {
    expect(run(false, [{ type: "state-view", status: "available" }])).toBe(false)
    expect(run(true, [{ type: "state-view", status: "available" }])).toBe(true)
  })

  it("状态离开 available 复位(进入流水线/消费完毕,旧请求不跨轮残留)", () => {
    // 覆盖两层升级的全部非 available 状态:应用升级(idle/checking/
    // downloading/ready/failed)与 dsh 升级(idle/active/ready/failed)
    for (const status of ["idle", "checking", "downloading", "ready", "failed", "active"]) {
      expect(run(true, [{ type: "state-view", status }])).toBe(false)
    }
  })

  it("生命周期序列:置位 → 流水线消费(复位)→ 下一轮再请求(再置位)", () => {
    const events: RequestedEvent[] = [
      { type: "card-request" },
      { type: "state-view", status: "available" },
      { type: "state-view", status: "downloading" },
      { type: "state-view", status: "idle" },
      { type: "card-request" },
    ]
    expect(run(false, events)).toBe(true)
    // 请求后事件又消费完毕:复位,不残留
    expect(run(true, [events[0], ...events.slice(1), { type: "state-view", status: "idle" }])).toBe(false)
  })

  it("全组合扫描:任意两事件序列归约与规则推导一致(置位/复位无遗漏)", () => {
    const statuses = ["idle", "available", "checking", "downloading", "ready", "failed", "active"]
    const events: RequestedEvent[] = [
      { type: "card-request" },
      ...statuses.map((status) => ({ type: "state-view" as const, status })),
    ]
    for (const initial of [false, true]) {
      for (const e1 of events) {
        for (const e2 of events) {
          const result = run(initial, [e1, e2])
          // 期望由规则推导(与上述定向用例一致):card-request 置位;
          // 非 available 状态视图复位;available 保留上一轮结果
          let expected = initial
          for (const e of [e1, e2]) {
            expected = e.type === "card-request" ? true : e.status === "available" ? expected : false
          }
          expect(result).toBe(expected)
        }
      }
    }
  })
})
