# Changelog

本文件记录 DeepSeek Desktop 的所有重要变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

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

[0.2.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.2.0
[0.1.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.1.0
