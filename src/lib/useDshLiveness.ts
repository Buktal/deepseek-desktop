// dsh 存活态镜像(F2):dshUrl / dshDown / dshExitCode 三值 + 状态转移规则
// 从 useBoot 抽出——原先散布在 useBoot 的 5 处接线里,规则只活在行长注释
// 中;useBoot 瘦身为纯 boot 流水线镜像,两个概念各归一个 module。
//
// 状态转移规则(与旧 useBoot 内联实现逐字等价,#40):
// - url:dsh-url 事件与 boot 视图携带的 dshUrl 两条来源,后到者覆盖;只增
//   不清——URL 记录后就一直有效(旧值在新 URL 到达前仍是当前呈现),快照
//   携带的 URL 只是历史记录,不代表 dsh 重新变活
// - down:置位 = dsh-exited 事件(reaper 推 dsh 意外退出;Rust 侧已排除退出
//   流程 / 升级流水线在途,此处只承载真实意外退出);清除 = 新 boot 开始
//   (checking 阶段)与任何 dsh-url 事件推达(升级完成 / 返回 dsh 等恢复路径
//   = dsh 重新变活)
// - exitCode:随 dsh-exited 置位携带(缺省 null = 句柄异常等未知场景)
//
// 规则收在 reduceDshLiveness 纯函数里(可测);hook 只做接线:两个事件监听
// (走 useRustEvent 样板)+ useBoot 视图投影的入口 applyBootView。

import { useCallback, useReducer } from "react"

import { useRustEvent } from "@/lib/useRustEvent"

export interface DshLivenessState {
  /** 当前 dsh 页 URL(Rust record_dsh_url 单一事实来源;null = 未就绪) */
  url: string | null
  /** dsh 意外退出(reaper dsh-exited 事件置位;新 boot / 新 URL 推达清除) */
  down: boolean
  /** 意外退出的退出码(缺省 null = 句柄异常等未知场景) */
  exitCode: number | null
}

export type DshLivenessEvent =
  /** boot 状态视图投影(useBoot 的 applyView 转投;checking = 新 boot 开始) */
  | { type: "boot-view"; phase: string; dshUrl?: string | null }
  /** record_dsh_url 推达新 URL(单一事实来源,4 时点统一,ADR 0001) */
  | { type: "dsh-url"; url: string }
  /** reaper 推 dsh 意外退出(exitCode 缺省 = 未知场景) */
  | { type: "dsh-exited"; exitCode: number | null }

export function reduceDshLiveness(
  state: DshLivenessState,
  event: DshLivenessEvent,
): DshLivenessState {
  switch (event.type) {
    case "boot-view":
      return {
        ...state,
        // 快照/事件携带的 URL 只增不清:记录后一直有效(旧值在新 URL 到达前
        // 仍是当前呈现);快照 URL 不代表 dsh 重新变活,不清退出态
        url: event.dshUrl || state.url,
        // 意外退出覆盖层的 [重试] = 重跑 boot:新 boot 开始(checking)即清除
        down: event.phase === "checking" ? false : state.down,
        exitCode: event.phase === "checking" ? null : state.exitCode,
      }
    case "dsh-url":
      // 任何恢复路径(升级完成 / 返回 dsh / 重试就绪)推达新 URL = dsh 重新
      // 变活,意外退出态清除(否则覆盖层在恢复后仍占屏)
      return { url: event.url, down: false, exitCode: null }
    case "dsh-exited":
      return { ...state, down: true, exitCode: event.exitCode }
  }
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

/** dsh-url 载荷守卫:url 必须是字符串(事件可能携带畸形载荷,不应用)。纯函数,可测。 */
export function isDshUrlPayload(p: unknown): p is DshUrlPayload {
  return typeof (p as Partial<DshUrlPayload>)?.url === "string"
}

/** useBoot 转投的视图投影(applyView 里发给 liveness 的最小面) */
export interface BootLivenessView {
  phase: string
  dshUrl?: string | null
}

const INITIAL: DshLivenessState = { url: null, down: false, exitCode: null }

export function useDshLiveness() {
  const [state, dispatch] = useReducer(reduceDshLiveness, INITIAL)

  // dsh-url 事件:record_dsh_url 推 URL(单一事实来源,4 时点统一,ADR 0001);
  // 载荷守卫:url 必须是字符串(事件可能携带畸形载荷,不应用)
  useRustEvent(
    "dsh-url",
    (payload) => dispatch({ type: "dsh-url", url: payload.url }),
    isDshUrlPayload,
  )

  // dsh-exited 事件:reaper 推 dsh 意外退出(Rust 侧已排除退出流程 / 升级
  // 流水线在途)→ 置位 down,deriveOverlay 出全屏错误覆盖层(#32/#40);
  // exitCode 提取自带防御(非数字按 null = 句柄异常等未知场景)
  useRustEvent<DshExitedPayload>("dsh-exited", (payload) => {
    dispatch({
      type: "dsh-exited",
      exitCode: typeof payload?.exitCode === "number" ? payload.exitCode : null,
    })
  })

  // boot 状态视图投影:新 boot 开始 / 快照携带 URL 的规则收在 reducer 里
  const applyBootView = useCallback((view: BootLivenessView) => {
    dispatch({ type: "boot-view", phase: view.phase, dshUrl: view.dshUrl })
  }, [])

  return { url: state.url, down: state.down, exitCode: state.exitCode, applyBootView }
}
