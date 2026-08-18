// boot 流水线前端接线:监听事件 + 触发/重试(触发命令同时返回状态快照,一调两用)。
// 同步骨架(先注册监听、再拉快照、后到者覆盖)走共享的 useRustStateSync;
// 本 hook 只负责 boot 特有的部分:applyView 字段映射、错误转结构化形态(fatal)、
// 耗时锚点插值(useAnchoredClock,F5)、quit/retry、dsh 存活态镜像的视图投影
// (F2:dshUrl / dshDown / dshExitCode 已抽到 useDshLiveness,本 hook 只把
// boot 状态视图转投给它,不持有存活态)。
import { invoke } from "@tauri-apps/api/core"
import { useCallback, useEffect, useState } from "react"

import {
  rawErrorMessage,
  toStructuredError,
  type RustErrorPayload,
  type StructuredError,
} from "@/lib/error"
import type { InstallStage } from "@/lib/installStage"
import { useAnchoredClock } from "@/lib/useAnchoredClock"
import type { BootLivenessView } from "@/lib/useDshLiveness"
import { useRustStateSync } from "@/lib/useRustStateSync"

export type Phase = "idle" | "checking" | "installing" | "starting" | "ready" | "error"

/** Rust 侧 boot-state 事件/命令返回的线上契约(serde camelCase,缺省字段为 None) */
export interface BootStateView {
  phase: Phase
  error?: RustErrorPayload | null
  /** 安装模拟进度 0-100,仅 installing 携带;100 = npm 进程退出校准(Rust 侧语义) */
  progress?: number | null
  /** 安装子阶段键后缀(InstallStage 联合,见 src/lib/installStage.ts) */
  stage?: InstallStage | null
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

export function useBoot({
  onBootView,
  dshUrl,
}: {
  /** dsh 存活态镜像的视图投影入口(useDshLiveness.applyBootView):boot 状态
   *  视图转投给 liveness,checking 清除 / URL 记录的规则在 liveness 侧 */
  onBootView: (view: BootLivenessView) => void
  /** 当前 dsh URL(useDshLiveness 持有;就绪缺 URL 兜底重拉快照用) */
  dshUrl: string | null
}) {
  const [phase, setPhase] = useState<Phase>("idle")
  // 跨边界保持结构化形态,ErrorScreen 渲染时才翻译(语言切换不冻结旧文案)
  const [error, setError] = useState<StructuredError | null>(null)
  const [logs, setLogs] = useState<BootLog[]>([])
  // Node 检测结果:仅 checking 阶段有值(Rust 侧离开 checking 清空)
  const [nodeVersion, setNodeVersion] = useState<string | null>(null)
  // 安装进度与子阶段:仅 installing 阶段有值;100 = npm 退出校准
  const [progress, setProgress] = useState<number | null>(null)
  const [stage, setStage] = useState<InstallStage | null>(null)
  // 耗时显示(F5):吃事件携带秒数、吐每秒插值秒数;锚点重置 / 停表条件 /
  // 挂载补偿都收在 useAnchoredClock 里
  const { elapsedSecs, setBase } = useAnchoredClock({
    running: phase !== "idle" && phase !== "error",
  })

  /** 应用一份状态视图(事件或快照):阶段/错误/进度/耗时统一入口。
   *  logs 只有快照携带(事件载荷是快照子集,经 source 区分)。 */
  const applyView = useCallback(
    (view: BootStateSnapshot, source: "event" | "snapshot") => {
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
        setBase(view.elapsedSecs)
      }
      if (source === "snapshot") {
        setLogs(view.logs)
      }
      // dsh 存活态投影:checking 清除意外退出 / 快照 URL 记录的规则在
      // useDshLiveness 的 reducer 里(与旧内联接线逐字等价)
      onBootView({ phase: view.phase, dshUrl: view.dshUrl })
    },
    [onBootView, setBase],
  )

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

  // 就绪缺 URL 兜底:phase=ready 而 dshUrl 仍为空(挂载晚于就绪且 dsh-url
  // 事件已错过、或快照竞态)时重拉一次快照——快照携带 dshUrl,覆盖事件
  // 丢帧;一次触发即止(deps 不变不循环),正常路径永不触发
  useEffect(() => {
    if (phase === "ready" && dshUrl === null) refresh()
  }, [phase, dshUrl, refresh])

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
    elapsedSecs,
    retry,
    quit,
  }
}
