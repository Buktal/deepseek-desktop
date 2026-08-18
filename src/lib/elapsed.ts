// boot 耗时显示(#7):formatElapsed 把秒拆成 { 分, 秒 } 结构化数据(供文案
// 模板插值),formatClock 产出仪表中心的 chronometer 读数。
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

/** boot 仪表中心读数的「分:秒」格式(语言中立,秒位补零)。分钟位可超 59
 *  (boot 超小时按累计分钟走,不进位小时位)。 */
export function formatClock(secs: number): string {
  const { minutes, seconds } = formatElapsed(secs)
  return `${minutes}:${String(seconds).padStart(2, "0")}`
}

/** 按锚点插值当前秒数(useAnchoredClock 的耗时显示):锚点(baseSecs, atMs)
 *  来自最近一次事件携带的秒数与本地到达时刻,渲染时按 nowMs 插值——起点是
 *  Rust 真实流水线启动时刻,挂载晚于启动也不丢已过时间,每秒 tick 重渲染
 *  不漂移。 */
export function interpolateElapsed(baseSecs: number, atMs: number, nowMs: number): number {
  return baseSecs + Math.floor((nowMs - atMs) / 1000)
}
