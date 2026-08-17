# 壳页常驻 + iframe 嵌 dsh（放弃多 webview 叠层）

壳的界面（弹窗、更新提示、菜单条）要常驻并盖在 dsh Web UI 之上，而不再是「整窗互斥导航」。同窗口多 webview 叠层经调查三平台全部不可行：webview 级输入穿透 API 全平台缺失（tauri#10564 open）、Linux 多 webview 布局结构性损坏（tauri#10420 / wry#1745 open）、multiwebview 是 unstable feature 且官方无维护带宽（tauri#9611 维护者原话）。故采用单 webview：壳页（本地 origin）常驻，dsh 以跨源 iframe 嵌入其容器内。实测 dsh 全部响应无 X-Frame-Options / CSP frame-ancestors，headless Chromium 实嵌渲染通过（dsh UI 完整可交互）。

调查与实测报告（含全部来源链接）：`research/overlay-feasibility` 分支 `docs/research/overlay-feasibility.md`、`research/dsh-iframe-test` 分支 `docs/research/dsh-iframe-test.md`。

## Considered Options

- 多 webview 叠层（No-Go）：上述三个上游阻塞点。
- 双窗口叠加薄层：Wayland 无法自定位窗口，长期架构不可取。
- 本地反代剥帧头 + iframe：仅当上游 dsh 加帧头时才需要，暂不做。
- iframe 嵌入（选定）：穿透与 resize 跟随天然消失，无 unstable 依赖。

## Consequences

- Rust 侧 `on_navigation` / `on_new_window` 对 iframe 内行为不可依赖（Windows 两者均不触发、三平台分叉，wry#1593 open）：外链拦截移到页面层——`initialization_script_for_all_frames` 注入拦截脚本 + `window.parent.postMessage` 回壳页 + `tauri-plugin-opener` 开系统浏览器；壳页顶层导航拦截保留（防壳页自身被整窗导航走）。
- dsh URL 单一事实来源 `record_dsh_url` 不变；4 个端口变化时点（boot 就绪 / 升级链就绪 / 升级卡「稍后」重启 / 崩溃重试）统一改为「推 URL 给壳页 → set iframe.src」；`navigate_main_window` / `navigate_to_dsh` / `wait_navigate_signal_or_timeout` 退役。
- 上游耦合：dsh 将来若加 XFO / frame-ancestors，iframe 即失效——dsh 升级回归需检查响应头；回退预案为恢复整窗互斥导航（git 历史）。
- ACL 边界不变：维持零 remote capability，dsh（跨源 iframe）拿不到任何 Tauri API。
