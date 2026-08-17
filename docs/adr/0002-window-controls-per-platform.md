# 三平台窗口控制分治（macOS Overlay；Windows/Linux 保留系统标题栏）

窗口控制的动机是三平台视觉统一 + 标题栏承载壳菜单 + 摆脱系统限制。事实调查表明：「菜单与系统窗口按钮同行」仅 macOS Overlay 原生可行；Windows 全自绘必丢 Win11 Snap Layouts 悬停（tauri#4531，上游 WebView2 阻塞，短期无解）；Linux 自绘无阴影无圆角（tao#157 维护者确认，compositor 决定）且 Wayland drag region 多坑；「Windows 标题栏暗色刺眼」保留系统标题栏即有官方解（`theme` 跟随应用主题 → DWM 暗色模式，Win10 1809+，黑/白二色）。故每平台取痛点最小一侧：macOS `titleBarStyle: "Overlay"` + `hiddenTitle`（系统红绿灯与 28px 壳菜单条同行）；Windows/Linux 保留系统标题栏、`theme` 跟随应用主题，壳菜单条放内容区第一行。壳菜单条为同一 React 组件三平台复用。「扩展边框帧」（保留系统按钮 + 网页占标题栏区域）无官方路径（tauri#4973 维护者确认 Windows 缺 API），排除。同类 Tauri 应用 CC-Switch 为同一打法的先例。

「摆脱系统限制」的初始动机被事实推翻后放弃：丢 Snap Layouts / Linux 阴影圆角 / Wayland 坑的代价，高于三平台像素级统一的收益。

事实底座：`research/window-controls-facts` 分支 `docs/research/window-controls-facts.md`。

## Consequences

- 应用内主题切换需同步窗口 `theme`（Windows/macOS 生效；Linux 由 GTK 主题决定，无操作）。
- macOS Overlay 已知坑要在实现中处置：未聚焦不可拖（tauri#4316）、无官方红绿灯 safe-area 数值（用 `trafficLightPosition` 主动定位或经验值）、拖拽条勿带背景色块避免双层标题栏视觉。
- 平台禁用 drag region 必须完全不渲染属性（空字符串/"false" 仍触发 wry 检测）。
- window-state 维持排除 `DECORATIONS` / `VISIBLE` flags（现状已正确，勿加回）。
