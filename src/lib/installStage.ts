// 安装子阶段键后缀的线上契约(Q3):Rust 侧 boot-state / upgrade-state 的
// stage 字段串的枚举。原先 InstallProgress 用 string + defaultValue 兜底,
// Rust 新增 stage 时 UI 静默空白;收成字面量联合后,新增 stage 必须同步本
// 联合与 locale JSON(installStage.test.ts 守住键存在),不变量从注释/兜底
// 挪进编译器。

/** 安装子阶段键后缀(boot 与 dsh 升级链共用同一组 stage 串) */
export type InstallStage = "fetching" | "reifying" | "finishing"
