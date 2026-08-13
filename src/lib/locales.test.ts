import { describe, expect, it } from "vitest"

import { DEFAULT_LANGUAGE, resolveLanguage } from "@/i18n/languages"
import en from "@/locales/en.json"
import zh from "@/locales/zh.json"

describe("resolveLanguage", () => {
  it("maps zh* / ZH* to zh", () => {
    expect(resolveLanguage("zh-CN")).toBe("zh")
    expect(resolveLanguage("zh-Hant-TW")).toBe("zh")
    expect(resolveLanguage("ZH")).toBe("zh")
  })

  it("maps en* to en", () => {
    expect(resolveLanguage("en-US")).toBe("en")
    expect(resolveLanguage("en-GB")).toBe("en")
    expect(resolveLanguage("EN")).toBe("en")
  })

  it("falls back to zh for unsupported or absent locales", () => {
    expect(resolveLanguage("ja-JP")).toBe("zh")
    expect(resolveLanguage("fr-FR")).toBe("zh")
    expect(resolveLanguage(undefined)).toBe("zh")
    expect(resolveLanguage(null)).toBe("zh")
    expect(resolveLanguage("")).toBe("zh")
    expect(DEFAULT_LANGUAGE).toBe("zh")
  })
})

describe("locale key sets", () => {
  it("zh and en carry the exact same keys (add a language: keep parity)", () => {
    const keysOf = (o: Record<string, unknown>) => Object.keys(o).sort()
    expect(keysOf(en)).toEqual(keysOf(zh))
  })
})

describe("i18n module initializes in a plain node environment", () => {
  it("production import path does not throw and resolves the initial language", async () => {
    // 生产入口 src/main.tsx 以 `import "@/i18n"` 方式导入;vitest(node 环境、
    // 无 navigator)也必须能 import——「模块顶层不触碰浏览器全局」不变量。
    const { default: i18n } = await import("@/i18n")
    expect(["zh", "en"]).toContain(i18n.language)
    // 资源已内联就绪:翻译命中,而非返回键名
    expect(i18n.t("error.title")).not.toBe("error.title")
    expect(i18n.t("errors.unknown")).not.toBe("errors.unknown")
  })

  it("every boot phase has a title key (BootScreen render path)", async () => {
    const { default: i18n } = await import("@/i18n")
    for (const phase of ["idle", "checking", "installing", "starting", "ready"]) {
      const title = i18n.t(`boot.${phase}.title`)
      expect(title).not.toBe(`boot.${phase}.title`)
      expect(title).not.toBe("")
    }
  })
})
