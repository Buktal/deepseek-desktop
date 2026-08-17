// shell-dialog 事件载荷守卫测试:线上契约(字段 camelCase,可选字段缺省不出现,
// 按钮 id/label 必填)shape 校验
import { describe, expect, it } from "vitest"

import { isShellDialogRequest } from "@/lib/shellDialog"

describe("isShellDialogRequest", () => {
  it("accepts a well-formed dialog request", () => {
    expect(
      isShellDialogRequest({
        kind: "update-found",
        title: "发现新版 v0.5.0",
        message: "正文",
        notes: "- fix a",
        buttons: [
          { id: "upgrade", label: "升级", variant: "primary" },
          { id: "later", label: "稍后", variant: "ghost" },
        ],
      }),
    ).toBe(true)
  })

  it("accepts a close-ask request with remember label", () => {
    expect(
      isShellDialogRequest({
        kind: "close-ask",
        title: "关闭 DeepSeek Desktop?",
        buttons: [
          { id: "minimize", label: "最小化到托盘", variant: "primary" },
          { id: "quit", label: "退出应用", variant: "outline" },
          { id: "cancel", label: "取消", variant: "ghost" },
        ],
        rememberLabel: "记住我的选择",
      }),
    ).toBe(true)
  })

  it("accepts a toast request without buttons", () => {
    expect(
      isShellDialogRequest({ kind: "toast-up-to-date", message: "已是最新", buttons: [] }),
    ).toBe(true)
  })

  it("rejects non-objects and unknown shapes", () => {
    expect(isShellDialogRequest(null)).toBe(false)
    expect(isShellDialogRequest("dialog")).toBe(false)
    expect(isShellDialogRequest({})).toBe(false)
    expect(isShellDialogRequest({ kind: "update-found" })).toBe(false) // buttons 缺失
    expect(isShellDialogRequest({ kind: 42, buttons: [] })).toBe(false)
    expect(isShellDialogRequest({ kind: "update-found", buttons: "x" })).toBe(false)
  })

  it("rejects malformed buttons and wrong field types", () => {
    expect(
      isShellDialogRequest({ kind: "update-found", buttons: [{ id: "upgrade" }] }),
    ).toBe(false) // label 缺失
    expect(
      isShellDialogRequest({ kind: "update-found", buttons: ["upgrade"] }),
    ).toBe(false) // 非对象
    expect(
      isShellDialogRequest({ kind: "update-found", buttons: [], title: 42 }),
    ).toBe(false) // title 非字符串
    expect(
      isShellDialogRequest({ kind: "update-found", buttons: [], rememberLabel: 1 }),
    ).toBe(false)
  })
})
