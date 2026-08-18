// InstallStage 联合的不变量测试(Q3):联合的每个成员都必须在 zh/en 两份
// locale JSON 的 boot 与 upgrade 命名空间下有文案键——原先的 defaultValue
// 兜底让缺失键静默空白,现在由编译器(联合)与这份测试守住键存在。
import { describe, expect, it } from "vitest"

import en from "@/locales/en.json"
import zh from "@/locales/zh.json"
import type { InstallStage } from "@/lib/installStage"

// 联合成员的手工枚举(新增 stage 需同步本表 + InstallStage 联合 + locale)
const STAGES: InstallStage[] = ["fetching", "reifying", "finishing"]

describe("InstallStage locale keys", () => {
  it("boot 与 upgrade 命名空间下,每个 stage 键在 zh/en 都存在", () => {
    for (const prefix of ["boot", "upgrade"] as const) {
      for (const stage of STAGES) {
        const key = `${prefix}.installing.stage.${stage}`
        expect(zh[key as keyof typeof zh], `zh 缺 ${key}`).toBeTruthy()
        expect(en[key as keyof typeof en], `en 缺 ${key}`).toBeTruthy()
      }
    }
  })
})
