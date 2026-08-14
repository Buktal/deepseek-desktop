// Node 引导页(#13):node 缺失 / 版本不符时替代通用错误页。
// 展示版本要求 + 当前检测结果(未安装 / 版本不符)+「前往 Node.js 官网下载」+
// 重试/退出(复用错误页现有 retry/quit 结构,不另起炉灶)。
// 判定:isNodeGuideError(仅 NodeMissing / NodeVersionUnmet 两个 kind 走这里,
// 其余错误留 ErrorScreen 通用布局)——错误跨边界保持结构化形态,渲染时翻译。
// 版本要求文本单一事实源在 Rust(NODE_REQ,经错误数据 required 携带),
// 前端不复制规格,避免 zh/en 与 Rust 三处维护同一串。
// 官网打开用 tauri-plugin-opener(#5 已注册 + opener:default 权限,useUpdateCheck 同款)。
import { openUrl } from "@tauri-apps/plugin-opener"
import { Download } from "lucide-react"
import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import type { NodeGuideError } from "@/lib/error"

/** Node.js 官网下载页(语言中立主站 URL;打开系统浏览器,不经 webview 内嵌) */
const NODEJS_DOWNLOAD_URL = "https://nodejs.org/en/download"

export function NodeGuideScreen({
  error,
  retry,
  quit,
}: {
  error: NodeGuideError
  retry: () => void
  quit: () => void
}) {
  const { t } = useTranslation()
  const data = error.data ?? {}
  const missing = error.type === "NodeMissing"
  // Rust 侧契约保证这两个字段存在(required 两 kind 都带,current 仅版本不符);
  // 空串兜底只防线上契约意外漂移,渲染不崩
  const required = typeof data.required === "string" ? data.required : ""
  const current = typeof data.current === "string" ? data.current : ""
  const openDownload = useCallback(() => {
    void openUrl(NODEJS_DOWNLOAD_URL).catch(() => {})
  }, [])

  return (
    <main className="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-background text-foreground">
      <div className="max-w-md text-center">
        <h1 className="text-lg font-medium">{t("guide.title")}</h1>
        <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
          {t("guide.requirement", { required })}
        </p>
        <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
          {missing
            ? t("guide.current.missing")
            : t("guide.current.unmet", { current })}
        </p>
      </div>

      <div className="flex flex-col items-center gap-4">
        <Button size="lg" onClick={openDownload}>
          <Download />
          {t("guide.download")}
        </Button>
        <div className="flex items-center gap-3">
          <Button size="lg" onClick={retry}>
            {t("common.retry")}
          </Button>
          <Button variant="ghost" size="lg" onClick={quit}>
            {t("common.quit")}
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">{t("guide.afterInstallHint")}</p>
      </div>
    </main>
  )
}
