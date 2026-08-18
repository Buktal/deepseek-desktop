// dsh 存活态状态机的纯核心测试(F2):置位(dsh-exited)/ 清除(新 boot
// checking / 新 URL 推达)/ URL 记录(快照投影与 dsh-url 事件)规则全组合。
// 生产路径:useDshLiveness 的 reducer(接线在 hook 里),与旧 useBoot 内联
// 实现逐字等价(#40)。
import { describe, expect, it } from "vitest"

import {
  reduceDshLiveness,
  type DshLivenessEvent,
  type DshLivenessState,
} from "@/lib/useDshLiveness"

// 全空闲基线:无 URL、dsh 在跑、无退出码
const base: DshLivenessState = { url: null, down: false, exitCode: null }

function run(initial: DshLivenessState, events: DshLivenessEvent[]) {
  return events.reduce(reduceDshLiveness, initial)
}

describe("reduceDshLiveness", () => {
  it("dsh-exited 置位:down=true + 携带退出码(缺省 null = 未知场景)", () => {
    expect(run(base, [{ type: "dsh-exited", exitCode: 1 }])).toEqual({
      url: null,
      down: true,
      exitCode: 1,
    })
    expect(run(base, [{ type: "dsh-exited", exitCode: null }])).toEqual({
      url: null,
      down: true,
      exitCode: null,
    })
  })

  it("dsh-url 事件推达:记录 URL 并清除意外退出态(恢复路径 = dsh 重新变活)", () => {
    const down = run(base, [{ type: "dsh-exited", exitCode: 1 }])
    expect(run(down, [{ type: "dsh-url", url: "http://x" }])).toEqual({
      url: "http://x",
      down: false,
      exitCode: null,
    })
    // 未置位时同样记录 URL
    expect(run(base, [{ type: "dsh-url", url: "http://x" }])).toEqual({
      url: "http://x",
      down: false,
      exitCode: null,
    })
  })

  it("boot 视图携带 URL:记录 URL(快照投影,只增不清)", () => {
    expect(run(base, [{ type: "boot-view", phase: "checking", dshUrl: "http://x" }])).toEqual({
      url: "http://x",
      down: false,
      exitCode: null,
    })
  })

  it("boot 视图不带 URL:保留旧 URL(URL 记录后就一直有效)", () => {
    const withUrl = run(base, [{ type: "dsh-url", url: "http://x" }])
    expect(run(withUrl, [{ type: "boot-view", phase: "starting", dshUrl: undefined }])).toEqual({
      url: "http://x",
      down: false,
      exitCode: null,
    })
  })

  it("快照投影的 URL 不代表 dsh 重新变活:不清意外退出态", () => {
    const down = run(base, [{ type: "dsh-exited", exitCode: 1 }])
    expect(run(down, [{ type: "boot-view", phase: "ready", dshUrl: "http://x" }])).toEqual({
      url: "http://x",
      down: true,
      exitCode: 1,
    })
  })

  it("新 boot 开始(checking)清除意外退出态([重试] = 重跑 boot)", () => {
    const down = run(base, [{ type: "dsh-exited", exitCode: 1 }])
    expect(run(down, [{ type: "boot-view", phase: "checking" }])).toEqual({
      url: null,
      down: false,
      exitCode: null,
    })
    // checking 携带 URL:一并记录
    expect(run(down, [{ type: "boot-view", phase: "checking", dshUrl: "http://x" }])).toEqual({
      url: "http://x",
      down: false,
      exitCode: null,
    })
  })

  it("非 checking 的 boot 视图不清意外退出态(错误/待机阶段不误报恢复)", () => {
    const down = run(base, [{ type: "dsh-exited", exitCode: 1 }])
    for (const phase of ["idle", "installing", "starting", "ready", "error"]) {
      expect(run(down, [{ type: "boot-view", phase }])).toEqual(down)
    }
  })

  it("全组合扫描:状态 × 事件归约与规则推导一致(置位/清除无遗漏)", () => {
    const urls = [null, "http://x"]
    const downs = [false, true]
    const exitCodes = [null, 1]
    const events: DshLivenessEvent[] = [
      { type: "dsh-exited", exitCode: 1 },
      { type: "dsh-exited", exitCode: null },
      { type: "dsh-url", url: "http://x" },
      { type: "boot-view", phase: "checking" },
      { type: "boot-view", phase: "checking", dshUrl: "http://x" },
      { type: "boot-view", phase: "ready" },
      { type: "boot-view", phase: "ready", dshUrl: "http://x" },
    ]
    for (const url of urls) {
      for (const down of downs) {
        for (const exitCode of exitCodes) {
          const state: DshLivenessState = { url, down, exitCode }
          for (const event of events) {
            // 期望由规则推导(与上述定向用例一致):dsh-exited 置位;dsh-url
            // 推达 = 变活(URL + 清除);boot-view 记录 URL、checking 清除
            let expected: DshLivenessState
            if (event.type === "dsh-exited") {
              expected = { ...state, down: true, exitCode: event.exitCode }
            } else if (event.type === "dsh-url") {
              expected = { url: event.url, down: false, exitCode: null }
            } else {
              expected = {
                url: event.dshUrl || state.url,
                down: event.phase === "checking" ? false : state.down,
                exitCode: event.phase === "checking" ? null : state.exitCode,
              }
            }
            expect(reduceDshLiveness(state, event)).toEqual(expected)
          }
        }
      }
    }
  })
})
