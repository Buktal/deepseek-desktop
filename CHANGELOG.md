# Changelog

本文件记录 DeepSeek Desktop 的所有重要变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

## [0.1.0] - 2026-08-14

### Added

- **首次发布**:DeepSeek Desktop —— DeepSeek dsh(DeepSeek Harness)的桌面包装壳:托盘常驻、关闭三选对话框(退出应用 / 最小化到托盘 / 取消),启动时自动拉起 dsh 并导航到其 Web UI。
- **启动流水线**:检测 Node 与全局 dsh——全局已有 dsh(任意可用版本)直接用;完全缺失时自动 `npm install -g @deepseek-ai/dsh@latest`。
- **安装包内置 npm 离线缓存**:无网络依赖的首次安装(缓存命中秒级完成,缺失自动回退网络)。
- **应用自身升级**:Ed25519 签名校验的自动更新(启动探测 + 6 小时轮询 + 托盘手动检查),Windows NSIS 就地升级。
- **dsh 升级**:独立于应用升级的用户确认流程(托盘入口),升级不重装不强制。
- **界面 i18n**:zh / en 双语,跟随系统语言(中文系统默认中文)。
- **CI 发布流水线**:打 tag 自动构建、签名并发布 Release(含 latest.json 更新清单)。

[0.1.0]: https://github.com/Buktal/deepseek-desktop/releases/tag/v0.1.0
