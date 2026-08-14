// boot 流水线前端接线:监听事件 + 触发/重试(触发命令同时返回状态快照,一调两用)。
// 同步骨架(先注册监听、再拉快照、后到者覆盖)走共享的 useRustStateSync;
// 本 hook 只负责 boot 特有的部分:applyView 字段映射、错误转结构化形态(fatal)、
// 耗时锚点插值、quit/retry。
import { invoke } from "@tauri-apps/api/core"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { interpolateElapsed } from "@/lib/elapsed"
import {
  rawErrorMessage,
  toStructuredError,
  type RustErrorPayload,
  type StructuredError,
} from "@/lib/error"
import { useRustStateSync } from "@/lib/useRustStateSync"

export type Phase = "idle" | "checking" | "installing" | "starting" | "ready" | "error"

/** Rust 侧 boot-state 事件/命令返回的线上契约(serde camelCase,缺省字段为 None) */
export interface BootStateView {
  phase: Phase
  error?: RustErrorPayload | null
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
  // 本地时刻;渲染时按 Date.now() 插值(interpolateElapsed,每秒 tick 触发重渲染),
  // 显示不漂移——起点是 Rust 真实流水线启动时刻,挂载晚于启动也不丢已过时间
  const [elapsedSecs, setElapsedSecs] = useState<number | null>(null)
  const elapsedAnchor = useRef<{ baseSecs: number; atMs: number } | null>(null)
  const [elapsedTick, setElapsedTick] = useState(0)
  // 挂载时快照是否已是 ready:true = 本地页由升级流程导航回来(dsh 已在跑),
  // 升级卡片可展示(#5 路由);false = 本次挂载经历 boot 推进(全新启动),
  // ready 事件到达后 Rust 即将导航去 dsh 页,不得中途切升级卡片。
  const [mountSnapshotReady, setMountSnapshotReady] = useState(false)

  /** 应用一份状态视图(事件或快照):阶段/错误/进度/耗时统一入口。
   *  logs 与 mountSnapshotReady 只有快照携带(事件载荷是快照子集,经
   *  source 区分)。 */
  const applyView = useCallback((view: BootStateSnapshot, source: "event" | "snapshot") => {
    if (view.phase === "checking") {
      setLogs([]) // 全新一次 boot 的语义边界(Rust 侧同步清空缓冲)
    }
    setPhase(view.phase)
    setError(toStructuredError(view.error ?? null))
    // 进度/阶段仅 installing 携带;离开 installing 清空(升级卡片等其他界面不残留)
    if (view.phase === "installing") {
      setProgress(view.progress ?? null)
      setStage(view.stage ?? null)
    } else {
      setProgress(null)
      setStage(null)
    }
    // node 检测结果仅 checking 携带;其余阶段事件不带字段 → 清空
    setNodeVersion(view.nodeVersion ?? null)
    if (view.elapsedSecs != null) {
      setElapsedSecs(view.elapsedSecs)
      elapsedAnchor.current = { baseSecs: view.elapsedSecs, atMs: Date.now() }
    }
    if (source === "snapshot") {
      setLogs(view.logs)
      if (view.phase === "ready") setMountSnapshotReady(true)
    }
  }, [])

  const fail = useCallback((e: StructuredError) => {
    setPhase("error")
    setError(e)
  }, [])

  // 同步骨架走共享核心:先注册监听再触发(增量事件一个不丢);
  // 快照与事件来自同一状态,后到者覆盖,无竞态。
  // 触发/重试:invoke("boot") 触发流水线并返回状态快照(含最近日志);
  // Rust 侧 phase + BOOTING 守卫去重,StrictMode 双 invoke 无害。
  const { refresh } = useRustStateSync({
    event: "boot-state",
    snapshot: () => invoke<BootStateSnapshot>("boot"),
    apply: applyView,
    onError: (e, source) =>
      // 命令拒绝(IPC/命令面异常):detail 透传原始串,框架文案渲染时翻译
      fail({
        kind: "app",
        type: source === "listen" ? "BootListenFailed" : "BootRequestFailed",
        data: { detail: rawErrorMessage(e) },
      }),
  })

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
    return interpolateElapsed(a.baseSecs, a.atMs, Date.now())
  }, [elapsedSecs, elapsedTick])

  const retry = refresh
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
