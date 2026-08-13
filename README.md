# DeepSeek Desktop

DeepSeek Desktop 是 DeepSeek dsh(DeepSeek Harness)的桌面包装壳:托盘常驻、关闭三选对话框,启动时自动拉起 dsh 并导航到其 Web UI。

## dsh 的安装位置

- dsh 装在 **npm 全局**(不是应用自管的目录):首次启动时若全局没有 dsh,应用自动执行 `npm install -g @deepseek-ai/dsh`。安装包内置 npm 离线缓存,缓存命中秒级完成,缓存缺失自动回退网络。
- 全局已有 dsh(任意可用版本)时直接使用:不重装、不比较版本、不强制升级;升级是独立的用户确认流程。

## 卸载

- **卸载本应用不会卸载全局 dsh**:dsh 属于用户资产,不随壳卸载被删除。
- 需要一并移除 dsh 时,请手动执行:

  ```bash
  npm uninstall -g @deepseek-ai/dsh
  ```
