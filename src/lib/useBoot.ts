// boot 流水线前端接线:监听事件 + 触发/重试(触发命令同时返回状态快照,一调两用)。
// 同步骨架(先注册监听、再拉快照、后到者覆盖)走共享的 useRustStateSync;
// 本 hook 只负责 boot 特有的部分:applyView 字段映射、错误转结构化形态(fatal)、
// 耗时锚点插值、quit/retry、dsh 意外退出态(dsh-exited 事件 → dshDown,
// deriveOverlay 的 Error 输入,#40)。
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
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
  /** 当前 dsh 页 URL(就绪过即携带;壳页 set iframe.src 用) */
  dshUrl?: string | null
}

export interface BootLog {
  stream: "stdout" | "stderr"
  line: string
}

/** Rust 侧 dsh-url 事件载荷(record_dsh_url 推 URL 给壳页,见 dsh.rs) */
export interface DshUrlPayload {
  url: string
}

/** Rust 侧 dsh-exited 事件载荷(reaper 推 dsh 意外退出,见 dsh.rs;字段缺省
 *  时 exitCode 为 null——句柄异常等未知场景) */
export interface DshExitedPayload {
  exitCode?: number | null
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
  // dsh URL(壳页 set iframe.src;单一事实来源在 Rust record_dsh_url):
  // 快照携带 + dsh-url 事件两条来源,后到者覆盖(与 boot-state 同款竞态语义);
  // 只增不清——URL 记录后就一直有效,旧值在新 URL 到达前仍是当前呈现
  const [dshUrl, setDshUrl] = useState<string | null>(null)
  // dsh 意外退出(reaper dsh-exited 事件置位 → deriveOverlay 出全屏错误覆盖层,
  // #32/#40):升级流水线在途的杀进程由 Rust 侧抑制(不推事件),此处镜像判定
  // 的输入只承载真实意外退出。清除时机 = dsh 重新变活:新 boot 开始(checking
  // 阶段)与任何新 URL 推达(dsh-url,覆盖升级完成 / 返回 dsh 等恢复路径)
  const [dshDown, setDshDown] = useState(false)
  const [dshExitCode, setDshExitCode] = useState<number | null>(null)

  /** 应用一份状态视图(事件或快照):阶段/错误/进度/耗时统一入口。
   *  logs 只有快照携带(事件载荷是快照子集,经 source 区分)。 */
  const applyView = useCallback((view: BootStateSnapshot, source: "event" | "snapshot") => {
    if (view.phase === "checking") {
      setLogs([]) // 全新一次 boot 的语义边界(Rust 侧同步清空缓冲)
      // 意外退出覆盖层的 [重试] = 重跑 boot:新 boot 开始即清除退出态
      setDshDown(false)
      setDshExitCode(null)
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
    if (view.dshUrl) setDshUrl(view.dshUrl)
    if (view.elapsedSecs != null) {
      setElapsedSecs(view.elapsedSecs)
      elapsedAnchor.current = { baseSecs: view.elapsedSecs, atMs: Date.now() }
    }
    if (source === "snapshot") {
      setLogs(view.logs)
    }
  }, [])

  const fail = useCallback((e: StructuredError) => {
    setPhase("error")
    setError(e)
  }, [])

  // dsh-url 事件:record_dsh_url 推 URL(单一事实来源,4 时点统一,ADR 0001)。
  // 先于快照注册监听(与 boot-state 同款「先监听后快照,后到者覆盖」语义;
  // 快照遗漏的极窄竞态由下方就绪缺 URL 兜底覆盖)
  useEffect(() => {
    let alive = true
    let stop: (() => void) | undefined
    void listen<DshUrlPayload>("dsh-url", (e) => {
      if (!alive) return
      if (typeof e.payload?.url === "string") {
        setDshUrl(e.payload.url)
        // 任何恢复路径(升级完成 / 返回 dsh / 重试就绪)推达新 URL = dsh
        // 重新变活,意外退出态清除(否则覆盖层在恢复后仍占屏)
        setDshDown(false)
        setDshExitCode(null)
      }
    }).then((un) => {
      stop = un
      if (!alive) un()
    })
    return () => {
      alive = false
      stop?.()
    }
  }, [])

  // dsh-exited 事件:reaper 推 dsh 意外退出(Rust 侧已排除退出流程 / 升级
  // 流水线在途)→ 置位 dshDown,deriveOverlay 出全屏错误覆盖层(#32/#40)
  useEffect(() => {
    let alive = true
    let stop: (() => void) | undefined
    void listen<DshExitedPayload>("dsh-exited", (e) => {
      if (!alive) return
      setDshDown(true)
      setDshExitCode(typeof e.payload?.exitCode === "number" ? e.payload.exitCode : null)
    }).then((un) => {
      stop = un
      if (!alive) un()
    })
    return () => {
      alive = false
      stop?.()
    }
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

  // 就绪缺 URL 兜底:phase=ready 而 dshUrl 仍为空(挂载晚于就绪且 dsh-url
  // 事件已错过、或快照竞态)时重拉一次快照——快照携带 dshUrl,覆盖事件
  // 丢帧;一次触发即止(deps 不变不循环),正常路径永不触发
  useEffect(() => {
    if (phase === "ready" && dshUrl === null) refresh()
  }, [phase, dshUrl, refresh])

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
    dshUrl,
    dshDown,
    dshExitCode,
    retry,
    quit,
  }
}
