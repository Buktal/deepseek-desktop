// boot 全程耗时显示(#7):把秒拆成 { 分, 秒 } 结构化数据。
// 返回结构化数据而非拼好的串:文案模板在 locale JSON(boot.elapsed.sec/minSec),
// zh/en 各自负责「N 秒 / N 分 M 秒」的语序;本函数只做数值拆分,不参与文案。
// 非有限值守卫:异常输入按 0 处理,不给渲染层喂 NaN/Infinity。
// 生产路径:BootScreen 渲染耗时显示时调用。

export interface ElapsedParts {
  minutes: number
  seconds: number
}

export function formatElapsed(secs: number): ElapsedParts {
  const s = Number.isFinite(secs) ? Math.max(0, Math.floor(secs)) : 0
  return { minutes: Math.floor(s / 60), seconds: s % 60 }
}

/** 按锚点插值当前秒数(useBoot 的耗时显示):锚点(baseSecs, atMs)来自最近
 *  一次事件携带的秒数与本地到达时刻,渲染时按 nowMs 插值——起点是 Rust 真实
 *  流水线启动时刻,挂载晚于启动也不丢已过时间,每秒 tick 重渲染不漂移。 */
export function interpolateElapsed(baseSecs: number, atMs: number, nowMs: number): number {
  return baseSecs + Math.floor((nowMs - atMs) / 1000)
}
