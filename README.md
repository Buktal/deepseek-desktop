# DeepSeek Desktop

DeepSeek Desktop 是 [DeepSeek dsh](https://www.npmjs.com/package/@deepseek-ai/dsh)（DeepSeek Harness）的桌面客户端：托盘常驻、关闭三选对话框，启动时自动拉起 dsh 并导航到其 Web UI。应用自身与 dsh 包各自独立升级。

> 技术栈：Tauri v2 + React 19 + TypeScript + shadcn/ui。

## 系统要求

- Windows 10 / 11（安装包为 NSIS 安装程序）
- Node.js 22.19+ 或 24+（含 npm；dsh 运行需要；缺失时应用会提示并引导安装）

## 安装

1. 从 [Releases](https://github.com/Buktal/deepseek-desktop/releases) 下载最新版安装包（`DeepSeek.Desktop_<版本>_x64-setup.exe`）并运行。
2. 首次运行会**自动安装 dsh**：安装包内置 npm 离线缓存，缓存命中秒级完成；缓存缺失时自动回退网络下载。
3. 若提示需要 WebView2 运行时，安装包会一并处理（Windows 10/11 通常已内置）。

> **关于 Windows SmartScreen 提示**：本项目尚未购买代码签名证书，首次运行时 Windows SmartScreen 可能提示「未知发布者」——这属于正常现象。点击「更多信息」→「仍要运行」即可；应用自身升级使用的 Ed25519 签名与 SmartScreen 无关（前者保证更新文件完整性，后者是商业证书信任链）。

## 使用

- 启动应用后自动拉起 dsh 并进入其 Web UI。
- **托盘常驻**：关闭窗口时应用仍驻留系统托盘（也可在关闭对话框中选择真正退出）。托盘菜单提供：打开主窗口、主题切换、开机自启、检查更新（应用与 dsh 两层）、退出。
- 关闭主窗口时弹出三选对话框：**退出应用 / 最小化到托盘 / 取消**。

## 升级

- **应用自身升级**：自动检测（启动时 + 每 6 小时 + 托盘手动检查），发现新版后托盘图标出现徽标，点击即可升级，Windows 就地静默安装。更新文件经签名校验，未通过不会安装。
- **dsh 升级**：自动检测全局 dsh 是否有新版本（启动时 + 每 6 小时 + 托盘手动检查），发现新版后托盘图标出现徽标与「升级 dsh」入口，确认后自动完成全局升级并重启 dsh；升级失败保留当前版本，不影响使用。

## 卸载

- **卸载本应用不会卸载全局 dsh**：dsh 属于用户资产，不随客户端卸载被删除。
- 需要一并移除 dsh 时，请手动执行：

  ```bash
  npm uninstall -g @deepseek-ai/dsh
  ```

## 许可证

[MIT](./LICENSE) © cc one Contributors

[![LINUX DO](https://img.shields.io/badge/LINUX%20DO-%E7%A4%BE%E5%8C%BA%E8%AE%A4%E5%8F%AF-blue?style=flat-square&logo=linux)](https://linux.do)
