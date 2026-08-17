# 调查：同窗口「底层 remote dsh + 上层壳透明薄层」三平台可行性（wayfinder #26）

- 日期：2026-08-17
- 调研目的：判定 Tauri 2 下「同一窗口、底层显示 dsh Web UI（http://127.0.0.1:port，remote origin）+ 上层壳自己的透明薄层（标题栏/菜单/弹窗/更新提示）」在 Windows / macOS / Ubuntu 是否可行：多 webview 叠层与透明、输入穿透、resize 跟随、内存代价、降级备选、安全边界。
- 方法：一手来源优先——Tauri 官方文档（v2.tauri.app / docs.rs / 官方示例源码）、tauri-apps/tauri 与 tauri-apps/wry 的 GitHub issues/PRs（含 wry 作者与 Tauri 维护者原话）、Microsoft WebView2 官方文档；辅以本地参考项目 CC-Switch 源码核查。除标注「推断/待实测」外，所有结论均有来源链接。查不到的一律标注，未编造。
- 本项目锁定版本：tauri 2.11.5 / wry 0.55.1（`src-tauri/Cargo.lock`）。

---

## 最重要的发现（先说结论）

**主路径（同窗口多 webview 叠层 + 输入穿透）在当前 Tauri 2.11.5 下三平台全部 No-Go**，阻断点有三，且互相独立：

1. **webview 级输入穿透 API 不存在**（全平台）：`set_ignore_cursor_events` 只有窗口级（作用于整个应用窗口，穿透到其它应用，不是穿透到窗口内另一个 webview）。给 `Webview` 加该方法的 feature request [tauri#10564](https://github.com/tauri-apps/tauri/issues/10564) 场景与我们完全一致（"two webviews in one window, one is as a transparent overlay over the other... can't click through"），至今 open、零评论、无关联 PR。
2. **Linux 上同窗口多 webview 布局结构性损坏**：子 webview 被塞进窗口默认 `gtk::Box` 垂直堆叠，任意 position 与 `set_bounds` 均为 no-op（[tauri#10420](https://github.com/tauri-apps/tauri/issues/10420) open；wry 侧修复 PR [wry#1745](https://github.com/tauri-apps/wry/pull/1745) open 未合）。
3. **multiwebview 整体处于 `unstable` feature 且官方无维护带宽**：Tauri 维护者 FabianLars 原话 "i currently cannot ask anyone (including myself) to work on this or any other unstable flag related bugs"（[tauri#9611](https://github.com/tauri-apps/tauri/issues/9611)）。

**推荐转向备选 B（单 webview + iframe 嵌 dsh）**：壳页常驻、iframe 装 dsh，标题栏/菜单是盖在 iframe 上的 DOM 元素——输入穿透与 resize 跟随两个难题天然消失，无 unstable、无第二 webview。唯一硬前提是 dsh Web UI 响应不带 `X-Frame-Options`/`CSP frame-ancestors`（代码搜索未见设置，**需一条 curl 实测定案**，见 §5）。

---

## 1. 同窗口多 webview 叠层：API 现状与透明

### 1.1 API 形态（Rust-only，需 `unstable` feature）

官方示例 `examples/multiwebview/main.rs`（[源码](https://github.com/tauri-apps/tauri/blob/dev/examples/multiwebview/main.rs)，本文已核对当前 dev 分支）：

```rust
let window = tauri::window::WindowBuilder::new(app, "main").inner_size(w, h).build()?;
let _webview1 = window.add_child(
  tauri::webview::WebviewBuilder::new("main1", WebviewUrl::App(Default::default())).auto_resize(),
  LogicalPosition::new(0., 0.), LogicalSize::new(w / 2., h / 2.),
)?;
```

- 该 API 受 tauri crate 的 `unstable` feature 门控：[docs.rs WebviewBuilder](https://docs.rs/tauri/2/tauri/webview/struct.WebviewBuilder.html)（页面即标注 unstable feature）；维护者在 [tauri#10011](https://github.com/tauri-apps/tauri/issues/10011) 评论中确认多 webview 必须开 unstable。
- **tauri.conf.json 不支持多 webview**：`WindowConfig` 没有 `webviews` 字段（核对 [tauri-utils/src/config.rs](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-utils/src/config.rs) dev 分支，仅 data_directory 注释提及 webviews）。只能 Rust 代码创建。本项目 `create_main_window`（src-tauri/src/navigation.rs:116）目前走 `WebviewWindowBuilder::from_config`，改多 webview 需换成 WindowBuilder + add_child 全代码路径，窗口配置（含 tauri-plugin-window-state 的还原）要手动对齐。
- **z-order 无 API**：`Webview` 没有 z-order/置顶方法（[docs.rs Webview](https://docs.rs/tauri/2/tauri/webview/struct.Webview.html) 方法表已核对）；叠加顺序只能依赖 add_child 的创建顺序（隐式约定），`reparent` 只能换窗口。

### 1.2 透明 webview

- `WebviewBuilder::transparent(bool)` 存在，但 docs.rs 原文："Available on **crate feature `macos-private-api`** or non-macOS"——Windows/Linux 无需特殊 feature，**macOS 必须启用 `macos-private-api`**（使用私有 API，App Store 不可上架；本项目走自分发 + updater，此代价可接受）。
- 各平台「透明 webview 盖住下方内容」的实况：
  - **macOS**：可行。透明 webview 叠在原生内容之上被 wry 作者确认正确（[tauri#10155](https://github.com/tauri-apps/tauri/issues/10155)，amrbashir："The macOS behavior is the correct one... we add the webview as a subview on top of the window contentView"）。但同窗口多子 webview 有过「只渲染最后一个 child」的 bug（[tauri#11376](https://github.com/tauri-apps/tauri/issues/11376)），评论确认由 [PR #11616](https://github.com/tauri-apps/tauri/pull/11616) 修复，issue 仍 open；且 macOS multiwebview 有输入法双字符 bug（[tauri#8705](https://github.com/tauri-apps/tauri/issues/8705) open）与方向键异常（[tauri#10194](https://github.com/tauri-apps/tauri/issues/10194) open）。
  - **Windows**：半可用、坑多。#11376 评论（xuchaoqian）："On Windows, it only shows the last child initially and will display the other children after resizing"；#10011 报告多 webview 加载白屏，另有评论 "In my app, overlays are white and the entire window hangs"（[tauri#10011](https://github.com/tauri-apps/tauri/issues/10011) open）。WebView2 透明背景本身可用（透明窗口 artifacts 已在 WebView2 Runtime 144 修复，[WebView2Feedback#5492](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5492)），但多个 WebView2 控件同窗叠层的稳定性无官方保证。
  - **Linux**：**结构性不可用**。①布局根因：tauri-runtime-wry 在 Linux 分支用 `build_gtk(window.default_vbox())` 创建 `gtk::Box` 并 `pack_start`，GTK 把窗口均分给子 webview（垂直堆叠症状），`WebView::set_bounds` 仅当父容器是 `gtk::Fixed` 才生效，否则 no-op（[#10420 根因分析评论](https://github.com/tauri-apps/tauri/issues/10420)）。wry 已合「支持加入 gtk::Fixed」（[wry#1128](https://github.com/tauri-apps/wry/pull/1128)），但 tao/tauri 侧未接，且 Fixed 定位修复 [wry#1745](https://github.com/tauri-apps/wry/pull/1745) 仍 open。②X11 子 webview 透明：[wry#1139](https://github.com/tauri-apps/wry/issues/1139) open（"Child WebView doesn't support transparency"，修复尝试未落地）。③Debian GNOME 实测 X11 与 Wayland 布局均错（[tauri#13071](https://github.com/tauri-apps/tauri/issues/13071)，closed 为 #10420 重复；维护者："The example only works on x11 but even there the `unstable` flag's name checks out"）。④webview 叠原生内容在 GTK 窗口上会闪烁且"we can't really do anything to fix this"（[tauri#9220](https://github.com/tauri-apps/tauri/issues/9220)，wry 作者），佐证 GTK 栈对叠层合成的基础性困难。

### 1.3 备选路径「双窗口叠加」的代价

见 §5-C。

---

## 2. 输入穿透（主路径最硬的阻断点）

- **窗口级 API 存在但答非所问**：`Window::set_ignore_cursor_events`（JS：`getCurrentWindow().setIgnoreCursorEvents(ignore)`，[v2.tauri.app window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/)）作用于**整个应用窗口**——开启后点击穿透到桌面/其它应用，而不是穿透到同窗口内下层的 dsh webview。对同窗口叠层场景完全无用。
- **webview 级 API 不存在（全平台）**：[docs.rs Webview](https://docs.rs/tauri/2/tauri/webview/struct.Webview.html) 与 [JS webview API](https://v2.tauri.app/reference/javascript/api/namespacewebview/) 方法表均无此方法；feature request [tauri#10564](https://github.com/tauri-apps/tauri/issues/10564)（open，零评论）场景即本票原样。
- **透明像素自动穿透不可行**：wry 作者原话："electron's setIgnoreMouseEvents will ignore **all** mouse events and not just on transparent areas... Detecting whether an area within a window is transparent or not is a near impossible task, even electron gave up"（[tauri#2090](https://github.com/tauri-apps/tauri/issues/2090) closed，该 issue 即此话题的始祖）。
- **Electron 的 `forward` 选项没有等价物**：#2090 评论（Andrew-web-coder）："the `forward` option is missing. So set_ignore_cursor_events can be used but it cannot be disabled, because all mouse events are ignored and you can not detect if user hovers over transparent area"——即动态切换必须由**外部信号**驱动恢复（社区做法是截图 alpha 轮询 + 手动切换，同 issue LZQCN 给出带性能开销的 workaround）。
- **同窗口叠层下的理论 hack（均非官方能力，按本仓库规则属补丁型，仅记录不推荐）**：
  - 用 `Webview::hide()/show()`（Rust+JS 均有）：薄层 JS 命中测试后 invoke Rust 把整个 overlay webview hide，让点击落到底层 dsh；恢复需 Rust 侧 `cursor_position` 轮询驱动——引入轮询与闪烁风险。
  - 用 [`Webview::with_webview`](https://docs.rs/tauri/2/tauri/webview/struct.Webview.html) 取平台句柄：macOS 可对 WKWebView 所在 NSView 设 `ignoresMouseEvents`（理论可行、未验证）；Windows 的 `ICoreWebView2Controller` 与 Linux webkitgtk 均无对应的 per-webview 输入开关。
- **结论**：即使叠层渲染在某平台可用，薄层空白区的点击要么被薄层吞掉（dsh 收不到），要么靠 hide/轮询 hack。Q2 是主路径全平台 No-Go 的第一理由。

---

## 3. 窗口 resize / 最大化时薄层 bounds 跟随

- **自动跟随 API**：`Webview::set_auto_resize`（docs.rs："controls whether the webview grows/shrinks with the parent window"；JS `setAutoResize`）——官方 multiwebview 示例即用它。
- **手动跟随 API**：Rust `window.on_window_event(WindowEvent::Resized)` → `webview.set_bounds/set_size/set_position`；JS `getCurrentWindow().onResized(cb)`（另有 `onMoved`、`onScaleChanged` 处理移动与 DPI/多显示器变化，[window API 文档](https://v2.tauri.app/reference/javascript/api/namespacewindow/)）。最大化/还原归入 Resized，同一机制覆盖。
- **已知 bug（全部 open）**：
  - [tauri#9611](https://github.com/tauri-apps/tauri/issues/9611)：`auto_resize` 与固定 offset position 不兼容——auto_resize 下 webview 的 position 不被固定；维护者明确无人力修 unstable 相关 bug（见 §开头引文）。
  - [tauri#10131](https://github.com/tauri-apps/tauri/issues/10131)：multiwebview 示例多次宽窄 resize 后 webview 停止跟随（宽度冻结，高度仍跟）。
  - **Linux：`set_bounds` 本身 no-op**（#10420 根因，见 §1.2）——跟随机制在 Linux 完全失效。
- **结论**：Windows/macOS 上「条状不透明薄层」可先用 auto_resize（接受 #9611/#10131 的边缘 bug）或 onResized 手动同步；Linux 两者皆不可用。

---

## 4. 常驻第二个 webview 的内存与性能代价

- **Windows / WebView2**（官方[进程模型文档](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model)）：同一 environment（同一 user data folder）下**单 browser 进程** + 共享 GPU/audio 等辅助进程；renderer 按源隔离——原文 "creating a new WebView2 instance from the same `CoreWebView2Environment`, but with a different domain in the `Source` property, **will usually start a new renderer process**"。本项目壳页（tauri 本地 origin）与 dsh（http://127.0.0.1:port）不同源 → 增量 ≈ 一个额外 renderer 进程（空载 Chromium renderer 数十 MB 量级，此数字为经验估计，官方未给）。
- **macOS / WKWebView**：同一 process pool 的多个 WKWebView 复用 WebKit WebContent 进程（引擎已知行为；Apple 文档本次抓取失败，此句置信度中等，方向为增量小于 Windows）。
- **Linux / WebKitGTK**：同一 WebKitWebContext 的多个 WebKitWebView 共享 WebKitWebProcess（引擎已知行为，置信度同上）。
- **合成性能**：叠层透明合成（尤其 Windows 两个 WebView2 控件同窗）存在额外合成成本与已知白屏/渲染顺序 bug（§1.2），无公开量化数据。
- **定性结论**：内存增量主要是 Windows 侧一个 renderer 进程；壳薄层是本地轻量页，渲染负载远小于 dsh 本身。真正的问题不是内存，而是 §1–§3 的功能正确性。

---

## 5. 平台判定与降级备选

### 5.1 主路径逐平台判定

| 能力 | Windows | macOS | Linux (X11) | Linux (Wayland) |
|---|---|---|---|---|
| 同窗口多 webview（unstable） | 可创建，白屏/顺序 bug（#10011、#11376） | 可创建，「只渲染最后 child」已修（#11616），输入法双字符 bug open（#8705） | 布局损坏 open（#10420） | 同左，且更差（#13071） |
| 透明 webview 叠层 | 基本可用、artifacts 曾需 runtime 144（#5492） | 可用，需 `macos-private-api`（#10155） | 子 webview 透明 open（wry#1139） | 未验证，偏向不可用 |
| webview 级输入穿透 | **无 API**（#10564） | **无 API** | **无 API** | **无 API** |
| bounds 跟随 | auto_resize 有 bug（#9611/#10131） | 同左 | set_bounds no-op | 同左 |

**总判定：主路径三平台 No-Go。**「薄层透明 + 空白区穿透」是产品需求的核心组合，恰是当前 Tauri 全平台缺失的那块。

### 5.2 备选 B（推荐主线）：单 webview + iframe 嵌 dsh

壳页（tauri: origin）常驻为唯一 webview，`<iframe src="http://127.0.0.1:port">` 装 dsh Web UI：

- 输入穿透：天然解决——标题栏/菜单/更新提示是盖在 iframe 之上的 DOM 元素，pointer-events 语义即所见即所得；空白区即 iframe 本身。
- resize/最大化：天然解决——DOM 流式布局，无 bounds 跟随、无 unstable。
- 无第二个 webview → §1–§4 全部问题不存在，内存零增量。
- 安全边界：iframe 内仍是 remote origin，能力系统按 origin 门禁（见 §6）——不给任何 capability 配 `remote` 即可维持现状边界。
- **硬前提（待实测定案）**：dsh Web UI 响应不得带 `X-Frame-Options: DENY/SAMEORIGIN` 或 `CSP frame-ancestors`。对 deepseek-harness 仓库做代码搜索未见相关设置（弱证据，GitHub code search 覆盖有限）；实测命令：`curl -sI http://127.0.0.1:3080 | grep -i -E 'x-frame|content-security'`。dsh 将来若加响应头该路径即死，属上游依赖风险。
- 其它需实测项：iframe 内键盘快捷键/剪贴板/全屏行为、dsh 路由跳转对 iframe history 的影响、更新弹窗的 z-index 与模态。
- 拖拽区：标题栏元素在壳 DOM 上直接用 `data-tauri-drag-region`——参考项目 CC-Switch 即此做法（`D:\AI\GitHub\CC-Switch\src\App.tsx` 自绘三键 + `D:\AI\GitHub\CC-Switch\src\lib\platform.ts` 的 Wayland 开关，规避 tauri#13440）。

### 5.3 备选 A（中间态）：同窗口条状薄层（不叠层）

标题栏做成顶部窄条 webview、dsh 占其余区域，互不重叠——避开透明与穿透两题，bounds 用 auto_resize。但仍是 multiwebview：Linux 布局损坏（#10420）与 auto_resize/position 不兼容（#9611）依旧存在，三平台交付仍受阻。仅当 Linux 上游修复后可作演进形态。

### 5.4 备选 C（不推荐）：双窗口叠加

主窗口装 dsh + 透明置顶跟随窗口装薄层。窗口级 `setIgnoreCursorEvents` + `transparent` 可用（macOS 仍需私有 API），但代价大：移动/resize/最大化/多显示器/DPI 全靠 `onMoved`/`onResized` 手动同步；Wayland 协议不允许客户端自定位窗口（平台限制，跟随方案在 Ubuntu Wayland 会话直接失效，参考 #13071 中维护者对 Wayland 的态度）；焦点抢夺、任务栏两条目、Alt-Tab 劣化。长期架构不推荐。

### 5.5 备选 D：维持现状（整窗互斥导航）

零新增风险，但「常驻标题栏/更新提示」需求不满足；作为 B 的实测失败后的兜底。

---

## 6. 安全边界核实（叠层/iframe 是否破坏 remote 隔离）

- 能力（capability）可直接按 webview 定向：`tauri-utils` 的 `Capability` 结构同时有 `windows` 与 `webviews` 字段（[capability.rs](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-utils/src/acl/capability.rs)，dev 分支 L163/L174），官方注释原文：
  - "If a window label matches any of the patterns in this list, the capability will be enabled on **all** the webviews of that window, regardless of the value of `Self::webviews`."
  - "**On multiwebview windows, prefer specifying `Self::webviews` and omitting `Self::windows` for a fine grained access control.**"
- remote origin 默认拿不到 API：官方[能力文档](https://v2.tauri.app/security/capabilities/)原文 "By default the API is only accessible to bundled code shipped with the Tauri App"；要放开必须显式配 `remote: { urls: [...] }`。
- **结论**：无论叠层还是 iframe，只要 dsh 的 origin（http://127.0.0.1:*）不出现在任何 capability 的 `remote.urls`，dsh 页就收不到任何 Tauri 事件/命令/插件权限——现有 ACL 拒绝行为不变，安全边界保持。若走多 webview，应把现有 `windows: ["main"]` 能力改为 `webviews: ["<壳webview标签>"]`（官方建议的细粒度写法）。
- iframe 附加注意：官方文档原话 "On Linux and Android, Tauri is unable to distinguish between requests from an embedded `<iframe>` and the window itself"——保守做法即零 remote capability（与现状一致），该限制即无实际影响。

---

## 7. 结论与建议

1. **Go/No-Go**：主路径（同窗口叠层薄层）当下三平台 No-Go——输入穿透 API 全平台缺失（[#10564](https://github.com/tauri-apps/tauri/issues/10564)）、Linux 布局根因未修（[#10420](https://github.com/tauri-apps/tauri/issues/10420) / [wry#1745](https://github.com/tauri-apps/wry/pull/1745)）、multiwebview 属 `unstable` 且官方无维护带宽（[#9611](https://github.com/tauri-apps/tauri/issues/9611)）。
2. **建议主线切换为备选 B（iframe 常驻壳）**：一步实测 `curl -I` 确认 dsh 无 `X-Frame-Options` 后即可进入实现评估；它同时满足常驻标题栏/菜单/更新提示与安全边界，且不依赖任何 unstable 能力。
3. **观察项**（若仍想走叠层）：订阅 #10564（穿透 API）与 wry#1745（Linux Fixed 定位）合入情况；两处合入前不建议重启主路径评估。
4. 参考 CC-Switch（同类 Tauri 应用）证明的可用形态是「单窗口 + `set_decorations(false)` + `data-tauri-drag-region` 自绘标题栏」——与备选 B 的标题栏实现路径一致，佐证其成熟度。

## 来源索引

官方文档：
- https://docs.rs/tauri/2/tauri/webview/struct.Webview.html（Webview 方法表：set_bounds/auto_resize/hide/show/with_webview，无 set_ignore_cursor_events）
- https://docs.rs/tauri/2/tauri/webview/struct.WebviewBuilder.html（transparent 的 macos-private-api 门控、unstable feature、initialization_script）
- https://v2.tauri.app/reference/javascript/api/namespacewindow/（setIgnoreCursorEvents、onResized/onMoved/onScaleChanged）
- https://v2.tauri.app/reference/javascript/api/namespacewebview/（Webview JS 方法表，无 setIgnoreCursorEvents）
- https://v2.tauri.app/security/capabilities/（remote 访问门禁、iframe/Linux 注意）
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model（WebView2 进程模型与 renderer 按源隔离）
- https://github.com/tauri-apps/tauri/blob/dev/examples/multiwebview/main.rs（官方 multiwebview 示例）
- https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-utils/src/acl/capability.rs（Capability.windows/webviews 字段及官方注释）

tauri/wry issues & PRs：
- https://github.com/tauri-apps/tauri/issues/10564（webview 级 set_ignore_cursor_events，open）
- https://github.com/tauri-apps/tauri/issues/10420（Linux 多 webview 布局损坏，open，含根因分析）
- https://github.com/tauri-apps/wry/pull/1745（gtk::Fixed 定位修复，open）
- https://github.com/tauri-apps/wry/pull/1128（支持 gtk::Fixed，merged）
- https://github.com/tauri-apps/tauri/issues/13071（Linux 布局错误，closed 为 #10420 重复）
- https://github.com/tauri-apps/tauri/issues/9611（auto_resize 与 position 不兼容 + 维护者无带宽原话，open）
- https://github.com/tauri-apps/tauri/issues/10131（resize 后停止跟随，open）
- https://github.com/tauri-apps/tauri/issues/11376（只渲染最后 child，部分由 #11616 修复）
- https://github.com/tauri-apps/tauri/issues/10011（多 webview 白屏，open）
- https://github.com/tauri-apps/tauri/issues/8705（macOS 双字符输入，open）
- https://github.com/tauri-apps/tauri/issues/10194（macOS 方向键异常，open）
- https://github.com/tauri-apps/tauri/issues/2090（透明区穿透不可行、无 forward 选项，closed）
- https://github.com/tauri-apps/tauri/issues/10155（macOS 透明 webview 叠原生可行）
- https://github.com/tauri-apps/tauri/issues/9220（GTK 上叠原生内容闪烁，wry 作者定性）
- https://github.com/tauri-apps/tauri/issues/8246（multiwebview 设计意图与 X11 限制，closed）
- https://github.com/tauri-apps/wry/issues/1139（X11 子 webview 透明，open）
- https://github.com/MicrosoftEdge/WebView2Feedback/issues/5492（透明 artifacts，runtime 144 修复）

本地核查：
- `D:\Project\O_DeepSeek_Desktop\src-tauri\Cargo.lock`（tauri 2.11.5 / wry 0.55.1）
- `D:\AI\GitHub\CC-Switch`（单窗口 + 自绘标题栏形态；无任何叠层/透明/穿透实现；`src-tauri/src/lib.rs`、`src/lib/platform.ts`、`src/App.tsx`）
