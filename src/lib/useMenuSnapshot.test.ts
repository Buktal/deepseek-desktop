// 菜单快照纯逻辑测试。生产路径:useMenuSnapshot 的 apply 经 isMenuSnapshot
// 校验后才写入状态;MenuBar 的徽标点经 menuHasBadge 判断。模块顶层不触碰
// window/document,纯 node 环境可 import(不变量,与 useThemeSync.test 同款)。
import { describe, expect, it } from "vitest"

import {
  isMenuSnapshot,
  menuHasBadge,
  type MenuItemView,
} from "@/lib/useMenuSnapshot"

describe("isMenuSnapshot", () => {
  it("accepts the contract shape (buttonLabel + items)", () => {
    expect(
      isMenuSnapshot({ buttonLabel: "菜单", items: [{ id: "toggle", label: "x", kind: "action" }] }),
    ).toBe(true)
    expect(isMenuSnapshot({ buttonLabel: "", items: [] })).toBe(true)
  })

  it("rejects non-objects and missing fields", () => {
    expect(isMenuSnapshot(undefined)).toBe(false)
    expect(isMenuSnapshot(null)).toBe(false)
    expect(isMenuSnapshot("menu-state")).toBe(false)
    expect(isMenuSnapshot(42)).toBe(false)
    expect(isMenuSnapshot({ items: [] })).toBe(false) // 缺 buttonLabel
    expect(isMenuSnapshot({ buttonLabel: "菜单" })).toBe(false) // 缺 items
    expect(isMenuSnapshot({ buttonLabel: "菜单", items: "not-array" })).toBe(false)
  })
})

describe("menuHasBadge", () => {
  const item = (overrides: Partial<MenuItemView>): MenuItemView => ({
    id: "x",
    label: "x",
    kind: "action",
    ...overrides,
  })

  it("is false when no item carries a badge", () => {
    expect(menuHasBadge([])).toBe(false)
    expect(menuHasBadge([item({})])).toBe(false)
    expect(
      menuHasBadge([
        item({}),
        item({
          kind: "submenu",
          children: [item({})],
        }),
      ]),
    ).toBe(false)
  })

  it("is true when any top-level item carries a badge", () => {
    expect(menuHasBadge([item({ badge: "0.5.0" })])).toBe(true)
  })

  it("finds badges nested in submenus", () => {
    expect(
      menuHasBadge([
        item({}),
        item({ kind: "submenu", children: [item({ badge: "0.1.0-rc.9" })] }),
      ]),
    ).toBe(true)
  })
})
