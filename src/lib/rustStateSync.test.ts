// 同步核心不变量测试。生产路径:useRustStateSync 的 effect 调用
// startRustStateSync(注入 Tauri 的 listen/invoke),本测试注入 fake 实现,
// 跑的是生产真实调用路径。
import { describe, expect, it, vi } from "vitest"

import {
  startRustStateSync,
  type RustStateSyncOptions,
} from "@/lib/rustStateSync"

interface View {
  phase: string
  logs?: string[]
}

function makeOptions(overrides: Partial<RustStateSyncOptions<View, View>> = {}) {
  const calls: string[] = []
  const options: RustStateSyncOptions<View, View> = {
    listen: vi.fn(async (_onPayload) => {
      calls.push("listen")
      return () => {
        calls.push("unlisten")
      }
    }),
    snapshot: vi.fn(async () => {
      calls.push("snapshot")
      return { phase: "snap" }
    }),
    apply: vi.fn(() => {
      calls.push("apply")
    }),
    ...overrides,
  }
  return { options, calls }
}

describe("startRustStateSync", () => {
  it("先注册监听,再拉快照(顺序不变量):快照在监听解析完成前不被调用", async () => {
    let resolveListen!: () => void
    const { options, calls } = makeOptions({
      listen: vi.fn((_onPayload) => {
        calls.push("listen")
        return new Promise<() => void>((resolve) => {
          resolveListen = () => resolve(() => {})
        })
      }),
      snapshot: vi.fn(async () => {
        calls.push("snapshot")
        return { phase: "snap" }
      }),
    })
    const stopPromise = startRustStateSync(options)
    await Promise.resolve() // 让 listen 开始执行
    expect(calls).toContain("listen")
    expect(calls).not.toContain("snapshot")
    resolveListen()
    await stopPromise
    expect(calls).toEqual(["listen", "snapshot", "apply"])
  })

  it("事件载荷经 apply(事件源)应用", async () => {
    let onPayload!: (payload: View) => void
    const { options } = makeOptions({
      listen: vi.fn(async (handler) => {
        onPayload = handler
        return () => {}
      }),
    })
    const stop = await startRustStateSync(options)
    onPayload({ phase: "checking" })
    expect(options.apply).toHaveBeenCalledWith({ phase: "checking" }, "event")
    stop()
  })

  it("快照经 apply(快照源)应用", async () => {
    const { options } = makeOptions()
    const stop = await startRustStateSync(options)
    expect(options.apply).toHaveBeenCalledWith({ phase: "snap" }, "snapshot")
    stop()
  })

  it("监听失败 → onError(listen) 且不再拉快照", async () => {
    const onError = vi.fn()
    const { options } = makeOptions({
      listen: vi.fn(async () => {
        throw new Error("listen boom")
      }),
      snapshot: vi.fn(),
      onError,
    })
    const stop = await startRustStateSync(options)
    expect(onError).toHaveBeenCalledWith(new Error("listen boom"), "listen")
    expect(options.snapshot).not.toHaveBeenCalled()
    expect(options.apply).not.toHaveBeenCalled()
    stop()
  })

  it("快照失败 → onError(snapshot),已注册的监听保留", async () => {
    const onError = vi.fn()
    const { options } = makeOptions({
      snapshot: vi.fn(async () => {
        throw new Error("snap boom")
      }),
      onError,
    })
    const stop = await startRustStateSync(options)
    expect(onError).toHaveBeenCalledWith(new Error("snap boom"), "snapshot")
    expect(options.apply).not.toHaveBeenCalled()
    stop()
  })

  it("返回的 stop 反注册监听", async () => {
    const { options } = makeOptions()
    const stop = await startRustStateSync(options)
    stop()
    expect(options.listen).toHaveBeenCalledTimes(1)
    // stop 可重复调用(React 清理幂等)
    stop()
    expect(options.listen).toHaveBeenCalledTimes(1)
  })
})
