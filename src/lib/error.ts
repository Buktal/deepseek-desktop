// 单一错误翻译接缝(O_CC_One 的 toStructuredError 模式照搬,适配本仓库判别式)。三层:
//   - `toStructuredError(e)` —— 把任意错误形态归约为 `StructuredError`,纯函数、不翻译。
//     后端 `{ kind, data }` BootError(经事件/命令返回,非 throw)保留为 `app` 形态,
//     可随时重译;其余形态(thrown `Error`、`{ message }`、裸字符串)坍缩为 raw 原串。
//   - `localizeStructuredError(s, t)` —— 渲染边界翻译:`app` → `errors.<type>` 键
//     (data 展开为插值变量);`raw` → 原样透传。
// 跨边界存储(useBoot 的 error 状态)保持结构化形态、渲染时才翻译,语言切换
// 会重新翻译而不是冻结旧语言的串。`describeError(e, t)` 组合二者,给「即取即译」
// 的调用点;找不到可识别内容时返回 "",调用方自行组合兜底(`errors.unknown`)。

import type { TFunction } from "i18next"

/** 结构化错误形态。`app` 携带后端 `{ kind, data }` 判别式(语言切换可重译);
 *  `raw` 是已定稿的串(thrown Error.message / 命令拒绝串),无翻译可做。 */
export type StructuredError =
  | { kind: "app"; type: string; data?: Record<string, unknown> }
  | { kind: "raw"; message: string }

/** Rust `BootError` 的结构化守卫(`{ kind: string, data?: object }`,
 *  unit 变体无 data 字段——serde tag/content 序列化形态)。 */
function isBootError(e: unknown): e is { kind: string; data?: Record<string, unknown> } {
  if (typeof e !== "object" || e === null) return false
  const rec = e as Record<string, unknown>
  if (typeof rec.kind !== "string") return false
  return rec.data === undefined || (typeof rec.data === "object" && rec.data !== null)
}

/**
 * 把任意错误归约为结构化形态——纯函数,不含翻译。`app` 形态保留判别式,
 * 供 `localizeStructuredError` 在语言切换后重译;其余形态坍缩为 raw 原串。
 * 无法识别时返回 null(调用方组合自己的兜底)。
 */
export function toStructuredError(e: unknown): StructuredError | null {
  if (isBootError(e)) return { kind: "app", type: e.kind, data: e.data }
  const message = rawErrorMessage(e)
  return message ? { kind: "raw", message } : null
}

/** 从非 BootError 形态抽取已定稿原串:thrown `Error.message`、普通对象的
 *  `.message` / `.data` / `.error` 字段,或裸字符串。无可识别内容返回 "". */
export function rawErrorMessage(e: unknown): string {
  if (e instanceof Error) return e.message
  if (e && typeof e === "object") {
    const m = e as Record<string, unknown>
    if (typeof m.message === "string") return m.message
    if (typeof m.data === "string") return m.data
    if (typeof m.error === "string") return m.error
  }
  return typeof e === "string" ? e : ""
}

/** 渲染边界翻译:`app` → 匹配的 `errors.<type>` 键(data 展开为插值变量;
 *  键缺失时返回 "",让调用方落兜底文案);`raw` → 原串不变。 */
export function localizeStructuredError(
  s: StructuredError,
  t: TFunction,
): string {
  if (s.kind === "raw") return s.message
  return t(`errors.${s.type}`, { ...s.data, defaultValue: "" })
}

/** 从未知错误抽取可读、已翻译的原因——「即取即译」调用点用,
 *  跨边界存储请用 `toStructuredError` 保持结构化形态。无可识别内容返回 "". */
export function describeError(e: unknown, t: TFunction): string {
  const s = toStructuredError(e)
  return s ? localizeStructuredError(s, t) : ""
}

/** Node 引导页错误形态:NodeMissing / NodeVersionUnmet 的 app 子集
 * (kind 判别 + data 载荷,渲染时经 guide.* 键翻译)。 */
export type NodeGuideError = {
  kind: "app"
  type: "NodeMissing" | "NodeVersionUnmet"
  data?: Record<string, unknown>
}

/** Node 引导页判定:NodeMissing / NodeVersionUnmet 属于「缺 Node / 版本不符」,
 *  由 NodeGuideScreen 引导(展示版本要求 + 当前检测结果 + 官网下载/重试);
 *  其余错误留通用错误页。纯函数、类型守卫,可测——判别与渲染分离。 */
export function isNodeGuideError(s: StructuredError | null): s is NodeGuideError {
  return (
    s !== null &&
    s.kind === "app" &&
    (s.type === "NodeMissing" || s.type === "NodeVersionUnmet")
  )
}
