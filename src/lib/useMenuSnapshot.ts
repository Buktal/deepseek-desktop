// 菜单快照的前端镜像(#33 拍板 / #38 施工):useRustStateSync 骨架接线,
// 事件 `menu-state` + 快照 `menu_snapshot` 命令(先注册监听再拉快照,后到者
// 覆盖);动作点击 → `menu_action` 命令回流,Rust 侧与托盘 on_menu_event 走
// 同一分发函数(两处入口、一张动作表)。
//
// 前端零菜单逻辑:勾选 / 禁用 / 动态升级条目 / 文案全部随快照(Rust locales
// 解析后放进快照,前端不放第二份文案表)。渲染是纯映射(DropdownMenu 组件),
// 本模块只提供快照 payload 守卫与徽标判断的纯函数,便于 vitest 测试。

import { invoke } from "@tauri-apps/api/core"
import { useCallback, useState } from "react"

import { useRustStateSync } from "@/lib/useRustStateSync"

export type MenuItemKind = "action" | "check" | "separator" | "submenu"

/** Rust 侧 MenuItem 的序列化形态(字段 camelCase,可选字段缺省不出现) */
export interface MenuItemView {
  id: string
  label: string
  kind: MenuItemKind
  checked?: boolean
  disabled?: boolean
  /** 升级徽标(待升级版本;菜单按钮徽标点据此显示,#3 §1 通知形态) */
  badge?: string
  children?: MenuItemView[]
}

/** Rust 侧 MenuSnapshot 的序列化形态(buttonLabel 由 Rust locales 解析) */
export interface MenuSnapshotView {
  buttonLabel: string
  items: MenuItemView[]
}

/** 快照 payload 守卫:shape 校验,未知值不应用到页面。纯函数,可测。 */
export function isMenuSnapshot(v: unknown): v is MenuSnapshotView {
  if (typeof v !== "object" || v === null) return false
  const snap = v as Record<string, unknown>
  return typeof snap.buttonLabel === "string" && Array.isArray(snap.items)
}

/** 任一菜单项(含嵌套子菜单)带徽标 → 菜单按钮显示徽标点。纯函数,可测。 */
export function menuHasBadge(items: readonly MenuItemView[]): boolean {
  return items.some(
    (item) =>
      item.badge !== undefined ||
      (item.children !== undefined && menuHasBadge(item.children)),
  )
}

export function useMenuSnapshot() {
  // 快照到达前:按钮只显示图标(无文案/无下拉),IPC 毫秒级后即完整
  const [snapshot, setSnapshot] = useState<MenuSnapshotView>({
    buttonLabel: "",
    items: [],
  })

  useRustStateSync({
    event: "menu-state",
    snapshot: () => invoke<MenuSnapshotView>("menu_snapshot"),
    apply: (view) => {
      if (isMenuSnapshot(view)) setSnapshot(view)
    },
    // 监听/快照失败:保持空快照(菜单按钮无下拉;托盘菜单不受影响,降级可用)
    onError: (e) => console.warn("[menu] 菜单快照同步失败", e),
  })

  // 动作回流:点击 → Rust 分发 → 状态变更 → 新快照随 menu-state 覆盖
  const dispatch = useCallback((id: string) => {
    void invoke("menu_action", { id }).catch((e) =>
      console.warn("[menu] 动作分发失败", e),
    )
  }, [])

  return { snapshot, dispatch }
}
