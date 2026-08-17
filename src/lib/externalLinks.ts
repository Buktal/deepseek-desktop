// 页面层外链拦截的消息校验(纯函数,可测):dsh iframe 内命中外链 →
// window.parent.postMessage({type:"open-external",url})(Rust 注入脚本见
// navigation.rs 的 EXTERNAL_LINK_SCRIPT)→ 壳页侧经本函数校验后交给
// tauri-plugin-opener 开系统浏览器。
// 协议白名单与 opener:default 权限一致(http/https/mailto/tel),其余一律
// 拒绝——校验不通过即静默丢弃(防御:iframe 侧消息不可信,只按契约放行)。
export const ALLOWED_EXTERNAL_SCHEMES = new Set(["http:", "https:", "mailto:", "tel:"])

/** 消息契约:open-external + url 字符串,URL 可解析且协议在白名单内。 */
export function parseExternalLinkMessage(data: unknown): string | null {
  if (typeof data !== "object" || data === null) return null
  const { type, url } = data as { type?: unknown; url?: unknown }
  if (type !== "open-external" || typeof url !== "string") return null
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return null
  }
  return ALLOWED_EXTERNAL_SCHEMES.has(parsed.protocol) ? parsed.href : null
}
