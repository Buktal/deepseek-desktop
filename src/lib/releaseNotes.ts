// release notes 摘要(update.notes 渲染前的派生逻辑):去 markdown 记号与
// 注释行,保留前几行。照搬 O_CC_One。纯函数,可测——原先埋在 UpdateCard
// 组件里不可测,归位到 lib(派生计算/格式化归 lib/)。

/** 预览保留的最大行数 */
export const NOTES_PREVIEW_LINES = 5

export function summarizeReleaseNotes(notes: string): string {
  return notes
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith("<!--"))
    .slice(0, NOTES_PREVIEW_LINES)
    .map((l) =>
      l
        .replace(/^[-*+]\s+/, "")
        .replace(/^#+\s*/, "")
        .replace(/[*_`]/g, "")
        .trim(),
    )
    .join("\n")
}
