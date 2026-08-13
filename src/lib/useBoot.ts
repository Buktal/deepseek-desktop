// boot 流水线前端接线:拉取状态快照 + 监听事件 + 触发/重试
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

  useEffect(() => {
    let mounted = true
    const unlisteners: UnlistenFn[] = []

    void (async () => {
      try {
        // 1. 初始快照(挂载时流水线可能已在运行,事件已错过,以快照为准)
        try {
          const snap = await invoke<BootStateSnapshot>("get_boot_state")
          if (!mounted) return
          setPhase(snap.phase)
          setError(snap.error ?? null)
          setLogs(snap.logs)
        } catch (e) {
          // IPC 失败:不静默停留在 loading,转可见错误(便于定位)
          if (mounted) {
            setPhase("error")
            setError(`无法连接桌面端服务,请重启应用。(${String(e)})`)
          }
          return
        }

        // 2. 增量事件(仅阶段;日志不推流,异常时经错误信息携带)
        const un1 = await listen<BootStateView>("boot-state", (e) => {
          if (e.payload.phase === "checking") {
            setLogs([]) // 全新一次 boot 的语义边界
          }
          setPhase(e.payload.phase)
          setError(e.payload.error ?? null)
        })
        unlisteners.push(un1)

        // 3. 触发流水线(Rust 侧 phase 守卫去重,StrictMode 双 invoke 无害)
        invoke("boot").catch(() => {})
      } catch (e) {
        // listen 注册失败
        if (mounted) {
          setPhase("error")
          setError(`事件监听初始化失败,请重启应用。(${String(e)})`)
        }
      }
    })()

    return () => {
      mounted = false
      unlisteners.forEach((u) => u())
    }
  }, [])

  const retry = useCallback(() => {
    invoke("boot").catch(() => {})
  }, [])
  const quit = useCallback(() => {
    invoke("quit_app").catch(() => {})
  }, [])

  return { phase, error, logs, retry, quit }
}
