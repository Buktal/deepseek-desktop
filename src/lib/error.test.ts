import type { TFunction } from "i18next"
import { describe, expect, it } from "vitest"

import {
  describeError,
  localizeStructuredError,
  rawErrorMessage,
  toStructuredError,
} from "@/lib/error"

// i18next `t` 的替身:回显键名 + 展开的插值变量(剥掉 defaultValue——生产里
// i18next 对已存在的键忽略它)。断言翻译路径选择了正确的键与载荷。
const t = ((key: string, opts?: Record<string, unknown>) => {
  const { defaultValue: _ignored, ...rest } = opts ?? {}
  const vars = Object.keys(rest)
  return vars.length ? `${key}:${JSON.stringify(rest)}` : key
}) as TFunction

describe("toStructuredError", () => {
  it("keeps a backend BootError as the re-translatable app shape", () => {
    expect(
      toStructuredError({ kind: "NodeCheckTimeout", data: { seconds: 10 } }),
    ).toEqual({ kind: "app", type: "NodeCheckTimeout", data: { seconds: 10 } })
  })

  it("keeps a unit-variant BootError (no data field)", () => {
    expect(toStructuredError({ kind: "NodeMissing" })).toEqual({
      kind: "app",
      type: "NodeMissing",
    })
  })

  it("collapses a thrown Error to its raw message", () => {
    expect(toStructuredError(new Error("boom"))).toEqual({
      kind: "raw",
      message: "boom",
    })
  })

  it("collapses a plain object / bare string to a raw message", () => {
    expect(toStructuredError({ message: "net down" })).toEqual({
      kind: "raw",
      message: "net down",
    })
    expect(toStructuredError("oops")).toEqual({ kind: "raw", message: "oops" })
  })

  it("returns null when nothing recognisable (no translation to defer)", () => {
    expect(toStructuredError(null)).toBeNull()
    expect(toStructuredError(undefined)).toBeNull()
    expect(toStructuredError({})).toBeNull()
    expect(toStructuredError(42)).toBeNull()
  })
})

describe("rawErrorMessage", () => {
  it("extracts .message from a plain object (command rejection shape)", () => {
    expect(rawErrorMessage({ message: "command rejected" })).toBe(
      "command rejected",
    )
  })

  it("extracts a string .data / .error field when there is no .message", () => {
    expect(rawErrorMessage({ data: "rate limited" })).toBe("rate limited")
    expect(rawErrorMessage({ error: "denied" })).toBe("denied")
  })

  it("returns a bare string verbatim and '' for nothing", () => {
    expect(rawErrorMessage("plain string")).toBe("plain string")
    expect(rawErrorMessage(null)).toBe("")
  })
})

describe("localizeStructuredError", () => {
  it("translates the app shape via errors.<type> with data interpolation", () => {
    expect(
      localizeStructuredError(
        { kind: "app", type: "NodeCheckTimeout", data: { seconds: 10 } },
        t,
      ),
    ).toBe('errors.NodeCheckTimeout:{"seconds":10}')
  })

  it("translates a unit-variant app shape with no data", () => {
    expect(localizeStructuredError({ kind: "app", type: "NodeMissing" }, t)).toBe(
      "errors.NodeMissing",
    )
  })

  it("passes the raw shape through unchanged (no translation)", () => {
    expect(localizeStructuredError({ kind: "raw", message: "boom" }, t)).toBe(
      "boom",
    )
  })
})

describe("describeError", () => {
  it("maps a structured BootError to errors.<type> with data interpolation", () => {
    expect(
      describeError({ kind: "NodeCheckTimeout", data: { seconds: 10 } }, t),
    ).toBe('errors.NodeCheckTimeout:{"seconds":10}')
    expect(describeError({ kind: "NodeMissing" }, t)).toBe("errors.NodeMissing")
  })

  it("extracts the message from a thrown Error (non-API path)", () => {
    expect(describeError(new Error("boom"), t)).toBe("boom")
  })

  it("returns a bare string verbatim", () => {
    expect(describeError("plain string", t)).toBe("plain string")
  })

  it("returns empty when nothing recognisable (caller adds fallback)", () => {
    expect(describeError(null, t)).toBe("")
    expect(describeError({}, t)).toBe("")
    expect(describeError(42, t)).toBe("")
  })
})
