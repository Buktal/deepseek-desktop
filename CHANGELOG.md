# Changelog

本文件记录 DeepSeek Desktop 的所有重要变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

## [1.0.0] - 2026-08-18

### Fixed

- **菜单勾选标记与文字对齐**(0.5.0 回归):开机自启 / 主题 / 关闭行为的勾选标记移行尾,所有菜单文字统一左对齐。

## [0.5.0] - 2026-08-18

### Added

- **iframe 壳页常驻**(#29 拍板 / #36 M1,ADR 0001):单 webview 内壳页常驻,dsh 以跨源 iframe 嵌入——六弹窗/更新提示/菜单条常驻盖在 dsh 之上;dsh URL 单一事实来源 `record_dsh_url` 在 4 个端口变化时点(boot 就绪 / 升级链就绪 /「稍后/返回」重启 / 崩溃重试)统一推给壳页 set iframe.src;整窗互斥导航退役;外链拦截移页面层(注入脚本拦截 `<a>` 点击 → postMessage → 系统浏览器)。
- **三平台窗口控制分治**(#28 拍板 / #37 M2,ADR 0002):macOS Overlay + 隐藏标题(系统红绿灯与 28px 壳菜单条同行);Windows/Linux 保留系统标题栏、`theme` 跟随应用主题;壳菜单条同一 React 组件三平台复用。
- **菜单快照与单一动作分发**(#38 M3):托盘与壳菜单条菜单同一份快照渲染(muda 投影),点击收敛到同一张动作表;「检查更新」disabled 随快照同步。
- **六弹窗 UI 化与更新进度浮层**(#39 M4):原生阻塞式 dialog 全部退役,改壳页 AlertDialog/toast(shell-dialog 事件);更新下载进度右下角浮层。
- **全屏覆盖层编排与旧组件 shadcn 化**(#40 M5):boot / dsh 升级 / dsh 意外退出三路状态镜像互斥覆盖层(deriveOverlay 纯函数,优先级 Error > Upgrade > Boot);意外退出转覆盖层 + [重试];前端组件统一到 shadcn/ui(FullScreenCard / ProgressRail 退役)。
- **上游耦合防线**(#41 M6):dsh 升级流水线新增响应头回归检查——新版 dsh 响应带 X-Frame-Options / CSP frame-ancestors 时报明确错误并指引回退预案(恢复整窗互斥导航,git 历史可回)。

### Fixed

- **npm-cache ETARGET 根治**(#41):内置离线缓存的 packument 早于升级目标版本时,`--prefer-offline` 跳过新鲜度检查导致 npm 误报「版本不存在」(ETARGET 假阴性);升级安装命中时回退无缓存网络重试一次(registry 数据恒新鲜)。

## [0.4.0] - 2026-08-17

### Added

- **跨平台发布**(#24):发布流水线从 Windows 单平台扩展为三平台矩阵(macOS Apple Silicon / Linux x64 / Windows x64,照搬 cc-one 已验证配置)——各平台 runner 独立构建安装包 + Ed25519 签名(.sig)并合并到同一 Release 的 latest.json 更新清单,应用内「检查更新」按平台检测对应安装包;npm 离线缓存(#16 约定)由每平台 runner 按构建平台解析原生可选依赖(如 esbuild),三平台缓存各得其所。
- **Unix 进程树杀**(#24):子进程 spawn 时统一放入新进程组,杀进程树与 Windows taskkill /T 对齐(macOS/Linux 下按进程组 SIGKILL),不再残留孤儿进程;npm 检测 / 安装 / dsh 启动全部覆盖。

### Changed

- **bundle.targets 改 all**(#24):tauri.conf.json 从 nsis 单目标扩展为三平台默认目标集——Windows 产出 NSIS(更新经 updaterJsonPreferNsis 走 NSIS,MSI 为附带产物)、macOS 产出 dmg + app、Linux 产出 deb + rpm + AppImage;updater 的 installMode 仍仅 Windows(passive NSIS),macOS/Linux 走 updater 插件默认形态。
- **macOS 托盘图标改 template 渲染**(#24,#3 遗留):菜单栏图标按 template(黑白,深浅菜单栏自动适配),Windows/Linux 维持彩色图标。

## [0.3.0] - 2026-08-14

### Changed

- **前端全量视觉审核**(#20):boot 启动页改「启动仪表」横排布局——左侧圆环仪表(刻度环 + 活动弧)、右侧阶段读数,进度轨与耗时读数对齐读数列下;升级/更新流程改收容卡片(bg-card + border),与 boot 开放画布区分;三处复制的滑动进度条归并为 ProgressRail 单组件,硬编码 indigo-500 全部收敛到 primary token(单一事实来源);暗色对比修复(emerald-600 / red-500 加 dark 变体);Node 引导页按钮层级(下载 primary / 重试 outline / 退出 ghost);reduced-motion 下停用圆环自转与滑动指示(阶段文本仍在)。
- **src 架构改进**(#21):四份手写「先注册监听、再拉快照、后到者覆盖」effect(useBoot / useDshUpgrade / useUpdateCheck / useThemeSync)归并为 rustStateSync 纯核心 + useRustStateSync hook 适配器,顺序不变量落进代码与单测;错误载荷接口归并为 RustErrorPayload;BootPhase 阶段联合类型单一来源;新增 interpolateElapsed / summarizeReleaseNotes 纯函数(单测覆盖生产路径);UpdateCard(原 UpgradeCard,消除与 UpgradeScreen 歧义)与 UpgradeScreen 共用 FullScreenCard 收容外壳;DownloadingBody 去掉 t prop 反模式,failed 态补 errors.unknown 兜底。
- **src-tauri/src 架构改进**(#22):模块拆分——dsh.rs 2013→1187 行,拆出 error.rs(结构化错误体系,DshError 原 BootError 归位改名)/ npm.rs(node 检测 + npm 全局安装域)/ proc.rs(子进程工具),导航执行归位 navigation.rs;单一事实来源——dsh URL 拼接 ×3 处 → dsh_url_for_port,npm 命令构造 ×2 处 → npm_command,非零退出 stderr detail 拼接 ×2 处 → exit_failure_detail;过时注释修正(IPC 命令面 2→3 个、boot_pipeline 步骤编号跳号);测试 70→72 全绿,cargo check / clippy 0 警告。

### Fixed

- **孤儿进程修复**(#22):子进程 ChildWaitError::Io 路径此前不杀子进程——node 检测 / npm root / 安装句柄异常时残留孤儿进程,且安装路径 install_pid 已清除、退出收敛也杀不到;抽 proc::kill_and_reap 统一 Timeout / Io 语义 + join 读线程。

## [0.2.0] - 2026-08-14

### Added

- **dsh 升级链**(#17,#3 设计落地):registry 直查检测(启动 + 每 6 小时 + 托盘「检查更新」,与应用升级共用触发)、托盘徽标 + 「升级 dsh」动态菜单提示,确认后全局升级 dsh 并自动重启回页面;失败保留当前版本,不影响使用。
- **Node 缺失引导页**(#13):检测结果可视化 + Node 官网下载引导 / 重试。
- **安装进度模拟与校准**(#7):installing 子阶段平滑推进 + npm 退出校准 100% + Windows 任务栏进度 + boot 全程耗时。
- **窗口状态记忆与开机自启**(#14):记忆窗口位置/尺寸,可随系统登录自动启动。
- **外部链接交给系统浏览器**(#15):应用内链接不再占用应用自身窗口。
- **boot 就绪退出过渡动画**(#4):启动完成时的过渡动画。

### Fixed

- **错误渲染修复**(#13 实机验证暴露):toStructuredError 对已归约 app 形态幂等——二次归约把 type 弄成 app,错误页所有 Rust 错误渲染成 errors.unknown。
- **README 重写**:用户向内容全面重写(措辞专业化、结构重组:系统要求 / 安装 / 使用 / 升级 / 卸载;不含 FAQ 与开发者章节)。
- **文档修正**:0.1.0 发布条目中的「dsh 升级」描述超前——该功能(托盘入口 + 用户确认流程)当时尚未落地,见 #17;README 已同步删除超前描述,功能落地后补写。

## [0.1.0] - 2026-08-14

### Added

- **首次发布**:DeepSeek Desktop —— DeepSeek dsh(DeepSeek Harness)的桌面包装壳:托盘常驻、关闭三选对话框(退出应用 / 最小化到托盘 / 取消),启动时自动拉起 dsh 并导航到其 Web UI。
- **启动流水线**:检测 Node 与全局 dsh——全局已有 dsh(任意可用版本)直接用;完全缺失时自动 `npm install -g @deepseek-ai/dsh@latest`。
- **安装包内置 npm 离线缓存**:无网络依赖的首次安装(缓存命中秒级完成,缺失自动回退网络)。
- **应用自身升级**:Ed25519 签名校验的自动更新(启动探测 + 6 小时轮询 + 托盘手动检查),Windows NSIS 就地升级。
- **dsh 升级**:独立于应用升级的用户确认流程(托盘入口),升级不重装不强制。
- **界面 i18n**:zh / en 双语,跟随系统语言(中文系统默认中文)。
- **CI 发布流水线**:打 tag 自动构建、签名并发布 Release(含 latest.json 更新清单)。

[1.0.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v1.0.0
[0.5.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.5.0
[0.4.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.4.0
[0.3.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.3.0
[0.2.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.2.0
[0.1.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.1.0
