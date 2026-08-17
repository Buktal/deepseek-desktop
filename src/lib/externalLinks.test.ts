// 页面层外链拦截消息校验测试(纯函数,生产路径即 useExternalLinks 的消费)
import { describe, expect, it } from "vitest"

import { parseExternalLinkMessage } from "@/lib/externalLinks"

describe("parseExternalLinkMessage", () => {
  it("accepts http(s) external urls", () => {
    expect(parseExternalLinkMessage({ type: "open-external", url: "https://github.com/Buktal/deepseek-desktop" })).toBe(
      "https://github.com/Buktal/deepseek-desktop",
    )
    expect(parseExternalLinkMessage({ type: "open-external", url: "http://example.com/a?b=1" })).toBe(
      "http://example.com/a?b=1",
    )
  })

  it("accepts mailto/tel (opener allow-default-urls)", () => {
    expect(parseExternalLinkMessage({ type: "open-external", url: "mailto:test@example.com" })).toBe(
      "mailto:test@example.com",
    )
    expect(parseExternalLinkMessage({ type: "open-external", url: "tel:+8613800000000" })).toBe(
      "tel:+8613800000000",
    )
  })

  it("rejects wrong message shape", () => {
    expect(parseExternalLinkMessage(null)).toBeNull()
    expect(parseExternalLinkMessage("open-external")).toBeNull()
    expect(parseExternalLinkMessage({ type: "other", url: "https://example.com" })).toBeNull()
    expect(parseExternalLinkMessage({ type: "open-external" })).toBeNull()
    expect(parseExternalLinkMessage({ type: "open-external", url: 42 })).toBeNull()
  })

  it("rejects unparsable or non-whitelisted schemes", () => {
    expect(parseExternalLinkMessage({ type: "open-external", url: "not a url" })).toBeNull()
    expect(parseExternalLinkMessage({ type: "open-external", url: "javascript:alert(1)" })).toBeNull()
    expect(parseExternalLinkMessage({ type: "open-external", url: "file:///C:/secret" })).toBeNull()
  })
})
