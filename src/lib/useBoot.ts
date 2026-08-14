// boot 流水线前端接线:监听事件 + 触发/重试(触发命令同时返回状态快照,一调两用)
import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { rawErrorMessage, toStructuredError, type StructuredError } from "@/lib/error"

export type Phase = "idle" | "checking" | "installing" | "starting" | "ready" | "error"

/** Rust 侧 BootError 的序列化形态(serde tag/content,unit 变体无 data 字段) */
export interface BootErrorPayload {
  kind: string
  data?: Record<string, unknown>
}

/** Rust 侧 boot-state 事件/命令返回的线上契约(serde camelCase,缺省字段为 None) */
export interface BootStateView {
  phase: Phase
  error?: BootErrorPayload | null
  /** 安装模拟进度 0-100,仅 installing 携带;100 = npm 进程退出校准(Rust 侧语义) */
  progress?: number | null
  /** 安装子阶段键后缀("fetching"|"reifying"|"finishing") */
  stage?: string | null
  /** Node 检测结果,仅 checking 阶段携带(启动页「检测到 Node.js vX」) */
  nodeVersion?: string | null
  /** 从流水线启动起的累计秒数(耗时显示起点 = Rust 真实启动时刻) */
  elapsedSecs?: number | null
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
  // Node 检测结果:仅 checking 阶段有值(Rust 侧离开 checking 清空)
  const [nodeVersion, setNodeVersion] = useState<string | null>(null)
  // 安装进度与子阶段:仅 installing 阶段有值;100 = npm 退出校准
  const [progress, setProgress] = useState<number | null>(null)
  const [stage, setStage] = useState<string | null>(null)
  // 耗时显示:elapsedSecs 存最近事件携带的秒数,elapsedAnchor 记下该事件到达的
  // 本地时刻;渲染时按 Date.now() 插值(每秒 tick 触发重渲染),显示不漂移——
  // 起点是 Rust 真实流水线启动时刻,挂载晚于启动也不丢已过时间
  const [elapsedSecs, setElapsedSecs] = useState<number | null>(null)
  const elapsedAnchor = useRef<{ baseSecs: number; atMs: number } | null>(null)
  const [elapsedTick, setElapsedTick] = useState(0)
  // 挂载时快照是否已是 ready:true = 本地页由升级流程导航回来(dsh 已在跑),
  // 升级卡片可展示(#5 路由);false = 本次挂载经历 boot 推进(全新启动),
  // ready 事件到达后 Rust 即将导航去 dsh 页,不得中途切升级卡片。
  const [mountSnapshotReady, setMountSnapshotReady] = useState(false)

  /** 应用一份状态视图(事件或快照):阶段/错误/进度/耗时统一入口 */
  const applyView = useCallback((v: BootStateView) => {
    if (v.phase === "checking") {
      setLogs([]) // 全新一次 boot 的语义边界(Rust 侧同步清空缓冲)
    }
    setPhase(v.phase)
    setError(toStructuredError(v.error ?? null))
    // 进度/阶段仅 installing 携带;离开 installing 清空(升级卡片等其他界面不残留)
    if (v.phase === "installing") {
      setProgress(v.progress ?? null)
      setStage(v.stage ?? null)
    } else {
      setProgress(null)
      setStage(null)
    }
    // node 检测结果仅 checking 携带;其余阶段事件不带字段 → 清空
    setNodeVersion(v.nodeVersion ?? null)
    if (v.elapsedSecs != null) {
      setElapsedSecs(v.elapsedSecs)
      elapsedAnchor.current = { baseSecs: v.elapsedSecs, atMs: Date.now() }
    }
  }, [])

  const fail = useCallback((e: StructuredError) => {
    setPhase("error")
    setError(e)
  }, [])

  // 触发/重试流水线,并应用命令返回的状态快照(含最近日志)。
  // Rust 侧 phase + BOOTING 守卫去重,StrictMode 双 invoke 无害。
  const trigger = useCallback(() => {
    void invoke<BootStateSnapshot>("boot")
      .then((snap) => {
        applyView(snap)
        setLogs(snap.logs)
        if (snap.phase === "ready") setMountSnapshotReady(true)
      })
      .catch((e) =>
        // 命令拒绝(IPC/命令面异常):detail 透传原始串,框架文案渲染时翻译
        fail({ kind: "app", type: "BootRequestFailed", data: { detail: rawErrorMessage(e) } }),
      )
  }, [applyView, fail])

  useEffect(() => {
    let mounted = true
    const unlisteners: UnlistenFn[] = []

    void (async () => {
      try {
        // 先注册监听,再触发:增量事件一个不丢;
        // 快照与事件来自同一状态,后到者覆盖,无竞态
        const un1 = await listen<BootStateView>("boot-state", (e) => {
          applyView(e.payload)
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
  }, [applyView, fail, trigger])

  // 耗时显示时钟:boot 阶段(含 ready 过渡)有秒数后每秒重渲染一次,渲染时
  // 按锚点插值(无 setTimeout 漂移);错误/待机页不显示耗时,停表免无谓的
  // 每秒重渲染(重试后新事件会重置起点并重启时钟)
  useEffect(() => {
    if (elapsedSecs === null || phase === "idle" || phase === "error") return
    const id = setInterval(() => setElapsedTick((n) => n + 1), 1000)
    return () => clearInterval(id)
  }, [elapsedSecs, phase])

  const displayElapsedSecs = useMemo(() => {
    if (elapsedSecs === null) return null
    const a = elapsedAnchor.current
    if (!a) return elapsedSecs
    return a.baseSecs + Math.floor((Date.now() - a.atMs) / 1000)
  }, [elapsedSecs, elapsedTick])

  const retry = useCallback(() => trigger(), [trigger])
  const quit = useCallback(() => {
    invoke("quit_app").catch(() => {})
  }, [])

  return {
    phase,
    error,
    logs,
    progress,
    stage,
    nodeVersion,
    elapsedSecs: displayElapsedSecs,
    retry,
    quit,
    mountSnapshotReady,
  }
}
