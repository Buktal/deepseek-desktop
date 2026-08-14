// Rust 状态镜像的 React 接线:把纯核心 startRustStateSync(rustStateSync.ts)
// 接到生命周期。外部资源(invoke / listen)全在 effect 内获取,node 环境可
// import,核心逻辑独立测试。配置走 ref:effect 只跑一次,调用方 inline 闭包
// 不引起监听重挂。
//
// 竞态语义(与旧四份手写 effect 一致):先注册监听,再拉快照,后到者覆盖;
// StrictMode 双挂载无害(监听注册/清理成对,快照拉取幂等)。
// 调用方职责:apply 负责把视图写进自己的 state;onError 决定失败是致命
// (boot:转结构化错误)还是降级(升级/主题:保持 idle)。

import { listen } from "@tauri-apps/api/event"
import { useCallback, useEffect, useRef } from "react"

import {
  startRustStateSync,
  type SyncErrorSource,
  type SyncSource,
} from "@/lib/rustStateSync"

export function useRustStateSync<S>(config: {
  /** 增量事件名(先注册监听,再拉快照,后到者覆盖) */
  event: string
  /** 快照命令(触发命令一调两用;boot 场景即 `boot` 命令本身) */
  snapshot: () => Promise<S>
  /** 每份视图(事件或快照)的应用入口 */
  apply: (view: S, source: SyncSource) => void
  /** 监听/快照失败的处理(默认静默) */
  onError?: (e: unknown, source: SyncErrorSource) => void
}): { refresh: () => void } {
  const configRef = useRef(config)
  configRef.current = config

  // 快照重拉(retry 用):不重挂监听,只重跑 snapshot——与挂载语义一致,
  // 后到者覆盖
  const refresh = useCallback(() => {
    const { snapshot, apply, onError } = configRef.current
    void snapshot().then(
      (snap) => apply(snap, "snapshot"),
      (e) => onError?.(e, "snapshot"),
    )
  }, [])

  useEffect(() => {
    let alive = true
    let stop: (() => void) | undefined
    void startRustStateSync({
      listen: (onPayload) =>
        listen<unknown>(configRef.current.event, (e) => onPayload(e.payload)).then(
          (un) => () => un(),
        ),
      snapshot: () => configRef.current.snapshot(),
      apply: (view, source) => {
        if (alive) configRef.current.apply(view, source)
      },
      onError: (e, source) => {
        if (alive) configRef.current.onError?.(e, source)
      },
    }).then((stopFn) => {
      stop = stopFn
      if (!alive) stopFn() // 清理先于异步解析完成:立即停,不残留监听
    })
    return () => {
      alive = false
      stop?.()
    }
  }, [])

  return { refresh }
}
