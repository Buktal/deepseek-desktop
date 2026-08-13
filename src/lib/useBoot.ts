// boot 流水线前端接线:监听事件 + 触发/重试(触发命令同时返回状态快照,一调两用)
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useCallback, useEffect, useState } from "react"

import { rawErrorMessage, toStructuredError, type StructuredError } from "@/lib/error"

export type Phase = "idle" | "checking" | "installing" | "starting" | "ready" | "error"

/** Rust 侧 BootError 的序列化形态(serde tag/content,unit 变体无 data 字段) */
export interface BootErrorPayload {
  kind: string
  data?: Record<string, unknown>
}

export interface BootStateView {
  phase: Phase
  error?: BootErrorPayload | null
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
  // 跨边界保持结构化形态,ErrorScreen 渲染时才翻译(语言切换不冻结旧文案)
  const [error, setError] = useState<StructuredError | null>(null)
  const [logs, setLogs] = useState<BootLog[]>([])

  const fail = useCallback((e: StructuredError) => {
    setPhase("error")
    setError(e)
  }, [])

  // 触发/重试流水线,并应用命令返回的状态快照(含最近日志)。
  // Rust 侧 phase + BOOTING 守卫去重,StrictMode 双 invoke 无害。
  const trigger = useCallback(() => {
    void invoke<BootStateSnapshot>("boot")
      .then((snap) => {
        setPhase(snap.phase)
        setError(toStructuredError(snap.error ?? null))
        setLogs(snap.logs)
      })
      .catch((e) =>
        // 命令拒绝(IPC/命令面异常):detail 透传原始串,框架文案渲染时翻译
        fail({ kind: "app", type: "BootRequestFailed", data: { detail: rawErrorMessage(e) } }),
      )
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
          setError(toStructuredError(e.payload.error ?? null))
        })
        unlisteners.push(un1)

        // 触发流水线 + 拉初始快照(挂载时流水线可能已在运行,事件已错过,以快照为准)
        if (mounted) trigger()
      } catch (e) {
        // listen 注册失败
        if (mounted)
          fail({
            kind: "app",
            type: "BootListenFailed",
            data: { detail: rawErrorMessage(e) },
          })
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
