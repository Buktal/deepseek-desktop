// 应用升级的展示纯函数与 UI 常量(Q2):updatePercent / RELEASES_URL 原先
// 埋在 useUpdateCheck.ts(组件被迫跨文件从 hook 文件取纯函数),归位到 lib/
// (派生计算/常量归 lib/,对齐 releaseNotes.ts 先例);hook 文件只留状态逻辑。

/** GitHub Releases 手动下载入口(失败降级,照搬 O_CC_One 的 RELEASES_URL) */
export const RELEASES_URL = "https://github.com/Buktal/deepseek-desktop/releases/latest"

/** 下载进度百分比(0-100)。total 未知(<=0)时返回 null,浮层显示「请稍候」。
 *  纯函数,可测。生产路径:UpdateFloat 的下载进度显示。 */
export function updatePercent(downloadedBytes: number, totalBytes: number): number | null {
  if (totalBytes <= 0 || downloadedBytes < 0) return null
  return Math.min(100, Math.round((downloadedBytes / totalBytes) * 100))
}
