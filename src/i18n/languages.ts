// 支持的语言注册表:新增语言的唯一入口。
// 加语言 = 在 src/locales/ 放 <code>.json + 在本文件注册 + 在 src/i18n/index.ts 注册资源。
// 键集一致性由 src/lib/locales.test.ts 守住(zh/en 键必须一一对应)。

export const SUPPORTED_LANGUAGES = ["zh", "en"] as const

export type SupportedLanguage = (typeof SUPPORTED_LANGUAGES)[number]

/** 兜底语言:系统语言既非 zh 也非 en 时使用(当前默认中文)。 */
export const DEFAULT_LANGUAGE: SupportedLanguage = "zh"

/**
 * 把任意 locale 串(如 navigator.language 的 "zh-CN" / "en-US")归约为支持的语言。
 * 规则:zh* → zh,en* → en,其余(含未定义)→ DEFAULT_LANGUAGE。
 * 与 Rust 侧 locales.rs 的 `lang_from_locale` 同规则(两个运行时各持一份,见该文件注释)。
 */
export function resolveLanguage(preferred: string | undefined | null): SupportedLanguage {
  if (!preferred) return DEFAULT_LANGUAGE
  const lower = preferred.toLowerCase()
  if (lower.startsWith("zh")) return "zh"
  if (lower.startsWith("en")) return "en"
  return DEFAULT_LANGUAGE
}
