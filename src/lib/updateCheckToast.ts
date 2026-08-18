// 手动「检查更新」的在途反馈:菜单条派发 check-update 即出 loading toast,
// 结果到达(五种 shell-dialog 事件,判别见 shellDialog.isCheckUpdateAnswer)
// 由 ShellDialogs 关闭——补上「点了菜单到结果弹出之间无任何反馈」的空档
// (慢网络下检查要数秒)。只覆盖菜单条路径:托盘触发时窗口可能隐藏,
// toast 不可见,不弹。15s 兜底超时防悬挂(正常路径结果必达,超时只是保险)。

import { toast } from "sonner"

/** loading toast 的稳定 id(ShellDialogs 按 id 定点关闭) */
const CHECK_UPDATE_TOAST_ID = "check-update-loading"

/** 兜底超时:超过此时长无结果事件则自动消失 */
const TIMEOUT_MS = 15_000

/** 菜单条「检查更新」点击时的在途提示(文案随调用方 i18n)。 */
export function showCheckUpdateLoading(message: string) {
  toast.loading(message, { id: CHECK_UPDATE_TOAST_ID, duration: TIMEOUT_MS })
}

/** 检查结果事件到达时关闭在途提示(幂等:未显示时无操作)。 */
export function dismissCheckUpdateLoading() {
  toast.dismiss(CHECK_UPDATE_TOAST_ID)
}
