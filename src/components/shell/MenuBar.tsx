// 壳菜单条(M2,#37 布局 + M3,#38 菜单;#42 三平台同行,ADR 0003)。
// - macOS:28px 拖拽条与系统红绿灯同行——data-tauri-drag-region 整行可拖
//   (drag.js 对 BUTTON 等可点击元素自动豁免,单击/双击最大化随 drag 内建),
//   左侧 84px 避让红绿灯(trafficLightPosition 已在 tauri.conf.json 定位);
//   无背景色块与分隔线(Overlay 下系统标题栏为透明覆盖,画了即双层视觉)。
// - Windows/Linux:无边框窗口一行——行本身为拖拽区(同上豁免与内建),
//   左侧菜单按钮,右侧自绘窗口控制贴右缘(WindowControls,#42)。
// - 菜单按钮(M3):快照渲染的 shadcn DropdownMenu——与托盘同源(useMenuSnapshot
//   镜像 Rust 的 MenuSnapshot),items 纯映射(check 行尾勾选 / disabled 禁用 /
//   submenu 嵌套 / separator 分隔),动作点击全部 invoke menu_action 回流
//   Rust 统一分发;升级槽位非空时按钮显示徽标点(与托盘徽标图标同源,#3 §1)。
//   勾选态直接取自快照、不维护本地切换状态:Rust 是事实源,新快照覆盖。
import { CheckIcon, MenuIcon } from "lucide-react"
import { useCallback } from "react"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { currentPlatform, dragRegionProps, menuBarLayout } from "@/lib/platform"
import { cn } from "@/lib/utils"
import {
  menuHasBadge,
  useMenuSnapshot,
  type MenuItemView,
} from "@/lib/useMenuSnapshot"
import { showCheckUpdateLoading } from "@/lib/updateCheckToast"
import { WindowControls } from "@/components/shell/WindowControls"

/** 快照 → DropdownMenu 纯映射(递归渲染子菜单)。 */
function MenuItems({
  items,
  dispatch,
}: {
  items: MenuItemView[]
  dispatch: (id: string) => void
}) {
  return (
    <DropdownMenuGroup>
      {items.map((item, index) => {
        const key = item.id || `sep-${index}`
        switch (item.kind) {
          case "separator":
            return <DropdownMenuSeparator key={key} />
          case "submenu":
            return (
              <DropdownMenuSub key={key}>
                <DropdownMenuSubTrigger>{item.label}</DropdownMenuSubTrigger>
                <DropdownMenuSubContent>
                  <MenuItems items={item.children ?? []} dispatch={dispatch} />
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            )
          case "check":
            return (
              <DropdownMenuItem
                key={key}
                disabled={item.disabled}
                onClick={() => dispatch(item.id)}
              >
                {item.label}
                {/* 勾选标记放行尾:所有菜单文字统一左起(勾选列在左会让该行
                    文字比普通项缩进);托盘原生菜单的勾选位由平台决定,不受此影响 */}
                {item.checked ? <CheckIcon className="ml-auto" /> : null}
              </DropdownMenuItem>
            )
          default:
            return (
              <DropdownMenuItem
                key={key}
                disabled={item.disabled}
                onClick={() => dispatch(item.id)}
              >
                {item.label}
              </DropdownMenuItem>
            )
        }
      })}
    </DropdownMenuGroup>
  )
}

export function MenuBar() {
  const platform = currentPlatform()
  const layout = menuBarLayout(platform)
  const { snapshot, dispatch } = useMenuSnapshot()
  const { t } = useTranslation()
  const hasBadge = menuHasBadge(snapshot.items)
  // 手动检查的在途反馈:派发 check-update 即出 loading toast,结果事件到达
  // 时由 ShellDialogs 关闭(菜单条路径窗口必可见;托盘路径不经过此处)。
  // id 契约:menu.rs 的 action 表("check-update",Rust 单一事实源)。
  const handleDispatch = useCallback(
    (id: string) => {
      if (id === "check-update") showCheckUpdateLoading(t("update.checking"))
      dispatch(id)
    },
    [dispatch, t],
  )
  return (
    <div
      {...dragRegionProps()}
      className={cn(
        "flex shrink-0 items-center",
        layout.heightClass,
        layout.paddingClass,
        layout.borderClass,
      )}
    >
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="sm"
              className="relative h-6 gap-1.5 px-2 text-xs"
            />
          }
        >
          <MenuIcon data-icon="inline-start" />
          {snapshot.buttonLabel}
          {/* 升级徽标点:任一菜单项带 badge 即显示(与托盘徽标图标同源,#3 §1) */}
          {hasBadge ? (
            <span
              aria-hidden
              className="absolute top-0.5 right-0.5 size-1.5 rounded-full bg-primary"
            />
          ) : null}
        </DropdownMenuTrigger>
        {/* 快照未到达(items 为空)时不渲染下拉:按钮保持占位(IPC 毫秒级) */}
        {snapshot.items.length > 0 ? (
          <DropdownMenuContent align="start" sideOffset={4} className="min-w-48">
            <MenuItems items={snapshot.items} dispatch={handleDispatch} />
          </DropdownMenuContent>
        ) : null}
      </DropdownMenu>
      {/* 自绘窗口控制(#42):仅 Windows/Linux,贴行右缘 */}
      {layout.windowControls ? <WindowControls /> : null}
    </div>
  )
}
