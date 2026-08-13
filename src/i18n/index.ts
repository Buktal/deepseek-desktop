// i18next 初始化。资源内联(单 bundle Tauri 应用,无懒加载需求)。
// 初始语言跟随系统语言(navigator.language,启动时读一次),fallback zh——
// boot 页首帧即正确语言,无闪烁。
// navigator 读取带环境守卫:vitest(node 环境)无 navigator 时回退默认 zh,
// 保证本模块可在纯 node 下 import(见 src/lib/locales.test.ts)。
// 语言偏好不持久化:当前无设置页;将来若加语言设置,以该设置为单一事实来源,
// 经 i18n.changeLanguage 跟随(O_CC_One 的 LanguageSync 模式)。

import i18n from "i18next"
import { initReactI18next } from "react-i18next"

import en from "@/locales/en.json"
import zh from "@/locales/zh.json"
import { resolveLanguage } from "@/i18n/languages"

const detected = typeof navigator !== "undefined" ? navigator.language : undefined

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    zh: { translation: zh },
  },
  lng: resolveLanguage(detected),
  fallbackLng: "zh",
  // 扁平点号键("boot.installing.hint")——层级活在键名里,不嵌套 JSON,
  // 语言文件保持扁平、可排序、便于键集一致性 diff。
  keySeparator: false,
  interpolation: { escapeValue: false },
  returnNull: false,
})

export default i18n
