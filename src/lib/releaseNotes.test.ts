// release notes 摘要的纯函数测试。生产路径:UpdateCard 的 available 卡片体
// 渲染 notes 时调用 summarizeReleaseNotes。
import { describe, expect, it } from "vitest"

import { summarizeReleaseNotes } from "@/lib/releaseNotes"

describe("summarizeReleaseNotes", () => {
  it("去掉列表记号与空行,保留行内内容", () => {
    expect(summarizeReleaseNotes("- first\n- second\n\n- third")).toBe(
      "first\nsecond\nthird",
    )
  })

  it("支持 * 与 + 列表记号", () => {
    expect(summarizeReleaseNotes("* one\n+ two")).toBe("one\ntwo")
  })

  it("去掉标题记号", () => {
    expect(summarizeReleaseNotes("# v1.0.0\n## Changes")).toBe("v1.0.0\nChanges")
  })

  it("去掉 markdown 强调记号(* _ `)", () => {
    expect(summarizeReleaseNotes("**bold** and *italic* and `code`")).toBe(
      "bold and italic and code",
    )
  })

  it("丢弃 HTML 注释行(<!-- 开头)", () => {
    expect(
      summarizeReleaseNotes("<!-- changelog comment -->\nreal line"),
    ).toBe("real line")
  })

  it("只保留前 5 行(预览行数上限)", () => {
    const lines = Array.from({ length: 8 }, (_, i) => `line ${i + 1}`)
    expect(summarizeReleaseNotes(lines.join("\n"))).toBe(
      ["line 1", "line 2", "line 3", "line 4", "line 5"].join("\n"),
    )
  })

  it("行首尾空白裁剪(去 markdown 缩进)", () => {
    expect(summarizeReleaseNotes("  indented  \n\t tabbed")).toBe("indented\ntabbed")
  })
})
