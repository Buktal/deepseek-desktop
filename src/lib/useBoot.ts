// boot 流水线前端接线:监听事件 + 触发/重试(触发命令同时返回状态快照,一调两用)
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useCallback, useEffect, useState } from "react"

export type Phase = "idle" | "checking" | "installing" | "starting" | "ready" | "error"

export interface BootStateView {
  phase: Phase
  error?: string | null
}

export interface BootStateSnapshot extends BootStateView {
  logs: BootLog[]
}

export interface BootLog {
  stream: "stdout" | "stderr"
  line: string
}

export function useBoot() {
  const [phase, setPhase] = useState<Phase>("idle")
  const [error, setError] = useState<string | null>(null)
  const [logs, setLogs] = useState<BootLog[]>([])

  const fail = useCallback((msg: string) => {
    setPhase("error")
    setError(msg)
  }, [])

  // 触发/重试流水线,并应用命令返回的状态快照(含最近日志)。
  // Rust 侧 phase + BOOTING 守卫去重,StrictMode 双 invoke 无害。
  const trigger = useCallback(() => {
    void invoke<BootStateSnapshot>("boot")
      .then((snap) => {
        setPhase(snap.phase)
        setError(snap.error ?? null)
        setLogs(snap.logs)
      })
      .catch((e) => fail(`启动请求失败:${String(e)}`))
  }, [fail])

  useEffect(() => {
    let mounted = true
    const unlisteners: UnlistenFn[] = []

    void (async () => {
      try {
        // 先注册监听,再触发:增量事件一个不丢;
        // 快照与事件来自同一状态,后到者覆盖,无竞态
        const un1 = await listen<BootStateView>("boot-state", (e) => {
          if (e.payload.phase === "checking") {
            setLogs([]) // 全新一次 boot 的语义边界(Rust 侧同步清空缓冲)
          }
          setPhase(e.payload.phase)
          setError(e.payload.error ?? null)
        })
        unlisteners.push(un1)

        // 触发流水线 + 拉初始快照(挂载时流水线可能已在运行,事件已错过,以快照为准)
        if (mounted) trigger()
      } catch (e) {
        // listen 注册失败
        if (mounted) fail(`事件监听初始化失败,请重启应用。(${String(e)})`)
      }
    })()

    return () => {
      mounted = false
      unlisteners.forEach((u) => u())
    }
  }, [fail, trigger])

  const retry = useCallback(() => trigger(), [trigger])
  const quit = useCallback(() => {
    invoke("quit_app").catch(() => {})
  }, [])

  return { phase, error, logs, retry, quit }
}
