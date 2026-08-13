# DeepSeek Desktop

DeepSeek Desktop 是 [DeepSeek dsh](https://www.npmjs.com/package/@deepseek-ai/dsh)(DeepSeek Harness)的桌面包装壳:托盘常驻、关闭三选对话框,启动时自动拉起 dsh 并导航到其 Web UI。应用自身与 dsh 包各自独立升级。

> 技术栈:Tauri v2 + React 19 + TypeScript + shadcn/ui。

## 用户指南

### 系统要求

- Windows 10 / 11(安装包为 NSIS 安装程序)
- Node.js 18+(含 npm;dsh 运行需要;缺失时应用会在启动时提示)

### 安装

1. 从 [Releases](https://github.com/Buktal/deepseek-desktop/releases) 下载最新版安装包(`DeepSeek-Desktop_<版本>_x64-setup.exe`)并运行。
2. 首次运行会**自动安装 dsh**:安装包内置 npm 离线缓存,缓存命中秒级完成;缓存缺失时自动回退网络下载。
3. 若提示需要 WebView2 运行时,安装包会一并处理(Windows 10/11 通常已内置)。

> **关于 Windows SmartScreen 提示**:本项目尚未购买代码签名证书,首次运行时 Windows SmartScreen 可能提示「未知发布者」——这属于正常现象。点击「更多信息」→「仍要运行」即可;应用自身升级使用的 Ed25519 签名与 SmartScreen 无关(前者保证更新文件完整性,后者是商业证书信任链)。

### 使用

- 启动应用后自动拉起 dsh 并进入其 Web UI。
- **托盘常驻**:关闭窗口时应用仍驻留系统托盘(也可在关闭对话框中选择真正退出)。托盘菜单提供:打开主窗口、检查应用更新、升级 dsh、退出。
- 关闭主窗口时弹出三选对话框:**退出应用 / 最小化到托盘 / 取消**。

### 升级

- **应用自身升级**:自动检测(启动时 + 每 6 小时 + 托盘手动检查),发现新版后托盘图标出现徽标,点击即可升级,Windows 就地静默安装。更新文件经签名校验,未通过不会安装。
- **dsh 升级**:独立于应用升级,通过托盘菜单触发,升级前会与你确认;升级使用 `npm install -g` 进行,失败会保留旧版,重试即可。
- 全局已有 dsh(任意可用版本)时直接使用,**不重装、不比较版本、不强制升级**。

### 卸载

- **卸载本应用不会卸载全局 dsh**:dsh 属于用户资产,不随壳卸载被删除。
- 需要一并移除 dsh 时,请手动执行:

  ```bash
  npm uninstall -g @deepseek-ai/dsh
  ```

### FAQ

- **Q:提示「未知发布者」/ SmartScreen 警告怎么办?** A:属正常(见上文安装说明),选择「仍要运行」即可。
- **Q:dsh 装在哪里?** A:装在 npm 全局目录(与命令行 `dsh` 命令共用一份,装一次两边可用)。
- **Q:为什么安装 dsh 会「秒级」完成?** A:安装包内置了 dsh 的完整依赖树离线缓存,无需现场下载 4 分钟。
- **Q:升级 dsh 会强制更新吗?** A:不会。全局已有可用版本就直接用,升级需你确认。
- **Q:应用离线能用吗?** A:首次安装 dsh 时可完全离线(内置缓存);之后 dsh 升级、应用升级需要联网。

## 开发者指南

### 架构

**Rust 后端**(`src-tauri/src/`):

- `lib.rs` — 组装:插件、单实例、dsh 生命周期、托盘、关闭三选对话框、退出收敛(杀 dsh 子进程)、生产日志
- `dsh.rs` — boot 流水线:检测 Node / 全局 dsh(有则用,无则装)→ 启动 dsh 服务 → 就绪后导航;安装包内置离线缓存的判定(`<资源目录>/npm-cache`,index-v5 标记)与 npm 参数组装
- `update.rs` — 应用自身升级(常驻):启动探测 + 6 小时轮询 + 托盘手动入口,下载 / 签名校验 / 安装 / 重启,状态机为纯函数
- `tray.rs` — 托盘菜单(跟随系统语言)、徽标图标
- `locales.rs` — Rust 侧原生界面文案(zh/en)
- `theme.rs`、`logging.rs` — 主题、日志落盘

**前端**(`src/`):

- `app/App.tsx` — 路由:应用升级卡片优先 → boot 分发 → dsh 页
- `lib/useBoot.ts`、`lib/useUpdateCheck.ts` — 镜像 Rust 状态的事件 hook
- `components/boot/`、`components/update/` — 启动页 / 错误页 / 升级卡片(shadcn/ui)
- `i18n/`、`locales/` — react-i18next,zh/en 键集一致性由单测守住

**关键设计**:

- dsh 装在 npm 全局,壳与命令行共享;壳卸载不卸 dsh(用户资产)
- 应用升级与 dsh 升级彻底解耦(壳不跟 dsh 发版)
- 检测时机:启动 + 定时(6h) + 托盘手动入口,两层升级共用
- 升级检查逻辑在 Rust 侧(不依赖前端页面存活),前端只镜像状态

### 构建

要求:Node.js 20+、Yarn 4(corepack)、Rust stable、Tauri CLI。

```bash
yarn install                 # 前端依赖(Yarn 4,corepack 已启用)
yarn web:dev                 # 仅前端开发
yarn tauri:dev               # 应用开发(自动拉起前端 dev server)
yarn web:test                # 前端单测(vitest)
cd src-tauri && cargo test   # Rust 单测
yarn tauri:build             # 生产构建(NSIS 安装包 + 签名产物)
```

`tauri build` 会自动执行 `beforeBuildCommand`(`yarn web:build && node scripts/prepare-npm-cache.mjs`),把 dsh 的离线缓存打进安装包。

### 发布

> 前提:仓库 Secrets 已配置签名密钥(`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,与 `tauri.conf.json` 中 `plugins.updater.pubkey` 对应;私钥不入库,见 #5)。密钥丢失或被篡改会破坏升级通道,务必妥善保管。

发一个版本的步骤:

1. **改版本号**(三处保持一致,语义化版本:`x.y.z`):
   - `package.json` → `version`
   - `src-tauri/tauri.conf.json` → `version`
   - `src-tauri/Cargo.toml` → `version`(改后跑一次 `cargo check`,让 `Cargo.lock` 同步)
2. **更新 CHANGELOG.md**(Keep a Changelog):把 `## [Unreleased]` 下的条目整理为新版本条目 `## [x.y.z] - 日期`,并在文末添加版本链接 `[x.y.z]: https://github.com/Buktal/deepseek-desktop/releases/tag/vx.y.z`。
3. **提交并推送**到 `main`(直接开发在 main,不建分支、不建 PR)。
4. **打 tag 并推送**(触发 CI 发布流水线):

   ```bash
   git tag v0.1.0 && git push origin v0.1.0
   ```

5. **监控 CI**:在 [Actions](https://github.com/Buktal/deepseek-desktop/actions) 查看 `Release` workflow(约 10-15 分钟)。成功后自动:
   - 构建并签名 NSIS 安装包(含内置 npm 离线缓存)
   - 发布 GitHub Release(正文来自 CHANGELOG 对应条目,即应用内更新说明)
   - 生成并上传 `latest.json`(应用升级清单,含 `.sig` 签名)
6. **验证**:检查 Release 资产齐全(安装包 + `latest.json` + `.sig`);可选:下载安装包实机安装,托盘「检查更新」应提示「已是最新版本」。

版本节奏:minor 功能、patch 修复;首发后常规语义化版本。
