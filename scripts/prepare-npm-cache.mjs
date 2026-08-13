// 把 @deepseek-ai/dsh@latest 的完整依赖树装进安装包内置 npm 离线缓存。
//
// 约定(#16,与 src-tauri/src/dsh.rs 的 bundle_cache_dir 对齐):
//   缓存目录 = <tauri.conf.json 所在目录>/resources/npm-cache,即构建后打进
//   bundle.resources 的 `<资源目录>/npm-cache`(npm cacache,index-v5 标记)。
// 运行时(见 dsh.rs):全局无 dsh 时 `npm install -g --prefer-offline --cache
// <目录> @deepseek-ai/dsh@latest` — 缓存命中秒级,缺失自动回退网络。
//
// 实现:把包装进临时 prefix(真实解析整个依赖树并下载全部 tarball 与
// 元数据进 cacache),仅保留缓存目录;临时 node_modules 安装完即删。
// 生成期间不打 --ignore-scripts:与运行时全局安装走同一条执行路径,
// 依赖树的 lifecycle 脚本能跑通本身就是对「缓存可离线安装」的证明。
//
// Windows 上 npm 是 .cmd shim,Node 22 起不能直接 spawn(.cmd EINVAL);
// 这里用 node 直接执行 npm-cli.js —— 与 npm.cmd 内部行为完全一致
// (官方安装布局固定为 <node根目录>/node_modules/npm/bin/npm-cli.js)。
//
// 失败即退出非零:构建时缓存目录缺失会令 tauri-bundler 报错,
// 本脚本保证该目录在 tauri build 之前就绪(beforeBuildCommand)。

import { execFileSync } from "node:child_process"
import { existsSync, mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"

const cacheDir = resolve("src-tauri", "resources", "npm-cache")
const tmpPrefix = mkdtempSync(join(tmpdir(), "dsh-bundle-cache-"))
const args = [
  "install",
  "--prefix", tmpPrefix,
  "--cache", cacheDir,
  "@deepseek-ai/dsh@latest",
  "--no-audit", "--no-fund", "--no-progress",
]

// Windows:node 直接跑 npm-cli.js;其余平台:直接 spawn npm。
let cmd, cmdArgs
if (process.platform === "win32") {
  cmd = process.execPath
  cmdArgs = [join(dirname(process.execPath), "node_modules", "npm", "bin", "npm-cli.js"), ...args]
  if (!existsSync(cmdArgs[0])) {
    throw new Error(`[prepare-npm-cache] 未找到 npm-cli.js: ${cmdArgs[0]}`)
  }
} else {
  cmd = "npm"
  cmdArgs = args
}

console.log(`[prepare-npm-cache] 生成离线缓存 → ${cacheDir}`)
try {
  execFileSync(cmd, cmdArgs, { stdio: "inherit" })
} finally {
  rmSync(tmpPrefix, { recursive: true, force: true })
}
