// Rust 状态镜像的纯函数核心(useRustStateSync 的接线对象,可测):
// 「先注册监听,再拉快照,后到者覆盖」——原先是 useBoot / useDshUpgrade /
// useUpdateCheck / useThemeSync 四份手写 effect 的同一模式(分叉,各改各的
// 迟早漂移),归并到这里成为唯一实现;顺序不变量(先监听后快照)直接写在代码
// 结构里(await listen 完成后才调 snapshot),由测试守住。
//
// 语义:
// - 监听注册失败 → onError(e, "listen"),不拉快照(与旧实现一致:try 整体跳出)
// - 快照失败 → onError(e, "snapshot"),已注册的监听保留
// - 事件载荷是快照的子集(如 boot 事件不带 logs):统一按 S 传给 apply,
//   消费方用 source 区分哪些字段只有快照携带
// - 返回 stop 函数(取消 + 反注册监听);调用后不得再调 apply——由
//   useRustStateSync 适配器的 alive 守卫保证

export type SyncSource = "event" | "snapshot"

export type SyncErrorSource = "listen" | "snapshot"

export interface RustStateSyncOptions<V, S extends V> {
  /** 增量事件监听器注册:返回反注册函数 */
  listen: (onPayload: (payload: V) => void) => Promise<() => void>
  /** 快照拉取(触发命令一调两用,如 boot 命令本身) */
  snapshot: () => Promise<S>
  /** 每份视图(事件或快照)的应用入口 */
  apply: (view: S, source: SyncSource) => void
  /** 监听/快照失败的处理(默认静默) */
  onError?: (e: unknown, source: SyncErrorSource) => void
}

export async function startRustStateSync<V, S extends V>(
  opts: RustStateSyncOptions<V, S>,
): Promise<() => void> {
  let stopListening: (() => void) | undefined
  try {
    stopListening = await opts.listen((payload) => {
      // 事件载荷按 S 处理(子集字段如 logs 缺失),消费方用 source 区分
      opts.apply(payload as S, "event")
    })
  } catch (e) {
    opts.onError?.(e, "listen")
    return () => stopListening?.()
  }
  try {
    const snap = await opts.snapshot()
    opts.apply(snap, "snapshot")
  } catch (e) {
    opts.onError?.(e, "snapshot")
  }
  return () => stopListening?.()
}
