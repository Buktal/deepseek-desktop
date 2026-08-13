import path from "node:path"
import { fileURLToPath } from "node:url"
import { defineConfig } from "vitest/config"

const __dirname = path.dirname(fileURLToPath(import.meta.url))

export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    // 纯函数测试为主,node 环境足够且更快;不做 DOM 渲染测试。
    include: ["src/**/*.test.ts"],
  },
})
