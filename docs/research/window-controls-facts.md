# 窗口控制路线事实清单:平台 × 路线能力对照(票 #27)

- 日期:2026-08-17
- 调研目的:为「窗口控制策略拍板」提供事实底座——decorations 关掉后各平台要补建什么、三条路线(①保留系统标题栏 / ②完全自绘 / ③macOS Overlay)的真实限制、Windows「扩展边框帧」有没有现成路径、window-state 插件的兼容坑。
- 来源分级(全文按此标注):
  - **[官方]** tauri.app 官方文档 / tauri-apps 官方仓库(tauri、tao、plugins-workspace)的源码注释与维护者表态——最高可信。
  - **[issue/PR]** tauri 系仓库的 issue/PR,标注 open/closed 与「维护者确认 / 单用户报告」。
  - **[社区]** 第三方插件/模板的 README 或作者表态——作者声称,未经官方背书。
  - **[本地参考]** `D:\AI\GitHub\CC-Switch` 源码实读(同类 Tauri 2 应用,本仓库 `lib.rs` 注释亦多处引用其做法)。
- 现状注明(未改动):本项目唯一窗口 `main`,tauri.conf.json 未设 `decorations`/`transparent`/`titleBarStyle`(=三平台系统标题栏),`shadow: true`;capabilities 仅 `core:default`(无任何窗口写权限);window-state 只持久化 `POSITION|SIZE|MAXIMIZED`;窗口由 `src-tauri/src/navigation.rs` 在 setup 中 `WebviewWindowBuilder::from_config` 创建(builder 需挂导航拦截,几何仍以 config 为单一事实来源)。
- 调研方式:官方文档 + 官方源码(dev 分支 ≈ 当前稳定线 2.11.x)逐条核实 + tauri 系 GitHub issues 检索 + CC-Switch 本地源码。**未做本机实验**,一切结论以来源为准。

---

## 〇、一页结论(浓缩对照表)

核心疑问先答:**「壳的菜单按钮与系统窗口按钮同行」只有 macOS 原生可行(Overlay);Windows/Linux 无任何官方或现成生态路径把网页内容放进系统标题栏行**(详见第三节)。想在这两个平台同行,只能 decorations:false 全自绘(代价见下表),或自写 Win32 子窗口(社区模板,无官方支持)。

| 能力/诉求 | ① 保留系统标题栏 | ② decorations:false 全自绘 | ③ macOS Overlay |
|---|---|---|---|
| 菜单按钮进标题栏行 | ✗ Win/Linux 无 API;macOS 菜单在全局菜单栏(不占窗口) | ✓ 三平台网页全占 | ✓ 仅 macOS |
| 暗色标题栏跟随应用主题 | Win ✓(仅黑/白二色,1809+);mac ✓(10.14+);Linux ✗(GTK 主题决定) | ✓ 自己画的 | 同①(macOS) |
| Win11 Snap Layouts(悬停最大化键) | ✓ | **✗ 丢失**(tauri#4531 open,上游 WebView2 阻塞);Win+方向键仍可用 | n/a |
| 标题栏右键系统菜单 | ✓ | ✗ 入口消失(Alt+Space 仍可用,需自绘右键菜单) | ✗ 同② |
| 圆角与阴影 | ✓ 系统 | Win ✓(shadow:true=1px 白边+Win11 圆角);mac ✓;**Linux ✗ 无阴影** | mac ✓ |
| 拖拽移动 | ✓ | 需自建 drag region(v2.11+ 支持 `"deep"`) | 需自建;未聚焦时不可拖(#4316 open) |
| 双击标题栏最大化 | ✓ | 内置于官方 drag 脚本,配好 drag region 即有 | 同左 |
| 视觉三平台统一 | ✗(各自系统样式) | ✓(代价=上表各行) | mac 与 ②/① 混搭 |
| 任务栏缩略图/点击最小化/还原 | ✓ | ✓(无已知损坏报告) | n/a |
| 实现工作量 | 0(现状)+可选 theme 配置 | 高(标题栏组件+权限+三平台回归) | 中(mac 分支处理) |

---

## 一、decorations:false 之后必须自建的能力清单

### 1.1 总表(能力 × 平台)

| 能力 | Windows | macOS | Linux |
|---|---|---|---|
| 拖拽移动 | 自建 drag region;坑少 | 自建;Overlay 下未聚焦不可拖(#4316) | 自建;**Wayland 问题多**,CC-Switch 直接全局禁用(本地参考) |
| 双击最大化 | drag region 自带 | drag region 自带(mouseup 且未移动才触发) | drag region 自带 |
| 窗口吸附 Snap | Snap Layouts 悬停弹出**丢失**;Win+方向键/拖边可用 | 无此概念(zoom 按钮兼全屏) | 由 WM 决定,基本不受影响 |
| 标题栏右键系统菜单 | 入口消失;Alt+Space 仍可用;需自绘菜单 | n/a(本无此菜单) | 入口消失;需自绘 |
| 圆角与阴影 | shadow:true→1px 白边+Win11 圆角 | 系统阴影/圆角默认保留 | **无阴影无圆角**(compositor 决定) |
| 任务栏缩略图与行为 | 正常(无已知 issue) | n/a | 正常 |
| 边缘拖拽 resize | ✓(tao 自动加透明 resize 子窗口) | startResizeDragging **不支持**(静默无效) | ✓ |
| 系统快捷键(Win+↑↓ 等) | ✓ 仍可用(shell 级) | — | — |

### 1.2 拖拽移动(data-tauri-drag-region 的能力与坑)

- **官方定义**:[官方文档](https://tauri.app/learn/window-customization/)——属性使元素成为拖拽区;**只在 mousedown 直接命中的元素上生效,子元素会拦截**(官方 Note 原文:"will only work on the element to which it is directly applied...preserved so that interactive elements like buttons and inputs can function properly")。
- **实现层**:不在 tao,而在 tauri 注入的 [drag.js](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/src/window/scripts/drag.js)。它内置可点击元素拦截清单(`A/BUTTON/INPUT/SELECT/TEXTAREA/LABEL/SUMMARY`、contenteditable、tabindex≥0、ARIA role button/link/menuitem/tab/checkbox/radio/switch/option)。
- **v2.11 起的官方解法**:`data-tauri-drag-region="deep"`(整个子树可拖)与 `"false"`(显式禁用),PR [tauri#15062](https://github.com/tauri-apps/tauri/pull/15062)(2026-03 合入;已逐 tag 核实 v2.10 无、v2.11 有)。**本仓库锁定 Tauri ≥2.11 即可绕开「子元素逐一加属性」的最大痛点**。官方 QA 示例:[examples/drag](https://github.com/tauri-apps/tauri/tree/dev/examples/drag)。
- **权限**:`core:window:default` **不含** start-dragging,需显式加 `core:window:allow-start-dragging`([官方权限示例](https://tauri.app/learn/window-customization/))。本项目当前 capabilities 只有 `core:default`,走自绘必须补。
- **已知坑**:
  - drag.js 的 `e.stopImmediatePropagation()` 会吃掉同元素上的鼠标事件监听: [#3811](https://github.com/tauri-apps/tauri/issues/3811)(open)、[#10767](https://github.com/tauri-apps/tauri/issues/10767)(open)。
  - Linux/Wayland: [#13440](https://github.com/tauri-apps/tauri/issues/13440)(拖拽触发窗口事件异常,CC-Switch 因此在 Linux 全局禁用 drag region)、[#12747](https://github.com/tauri-apps/tauri/issues/12747)(open,拖一下触发 blur+右键菜单)。
  - Windows 触屏: [#4746](https://github.com/tauri-apps/tauri/issues/4746)(open);官方 Tip——无需自定义交互时可用 `*[data-tauri-drag-region] { app-region: drag; }` 兼容触摸/笔。
  - 动态渲染(框架重建 DOM)下可能不生效: [#14450](https://github.com/tauri-apps/tauri/issues/14450)(closed)。
- **JS API**:`getCurrentWindow().startDragging()`;另有 `startResizeDragging(direction)`,**macOS 上 tao 返回 NotSupported 且 tauri 转发时吞错误(调用不报错但无效果)**——JS 官方文档未标注,只有 [tao 源码](https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/macos/window.rs)可证 [官方/源码]。

### 1.3 双击标题栏最大化

- **drag region 自带**,无需自己写:drag.js 头注释——"drag on mousedown and maximize on double click on Windows and Linux, while macOS maximization should be on mouseup and if the mouse moves after the double click, it should be cancelled"(macOS 行为对齐系统可取消语义,issue [#8306](https://github.com/tauri-apps/tauri/issues/8306) 已修)。双击走 `internal_toggle_maximize`,对应权限 `core:window:allow-internal-toggle-maximize` **已含于 `core:window:default`** [官方]。
- 手动实现参考(官方文档 "Manual Implementation" 示例):`mousedown` 时 `e.detail === 2 ? toggleMaximize() : startDragging()`。

### 1.4 窗口吸附(Snap)

- **Win11 Snap Layouts 悬停弹出:自绘最大化按钮必丢**。根因:弹出只能靠响应 `NC_HITTEST` 的 `HTMAXBUTTON`,而 webview 内的点击不产生该消息。官方维护者 FabianLars 明确 "This won't be implemented for v2 since we're still blocked by webview2":[tauri#4531](https://github.com/tauri-apps/tauri/issues/4531)(open,status: upstream;上游 [WebView2Feedback#3367](https://github.com/MicrosoftEdge/WebView2Feedback/issues/3367))。[维护者确认]
- **Win+↑↓←→ / 拖到屏幕边缘 snap / 拖边 resize:仍可用**——Win+方向键是 shell 级快捷键,tao 不拦截(社区报告);拖边 resize 由 tao 为无边框窗口自动创建透明子窗口处理([tauri-runtime-wry/undecorated_resizing.rs](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-runtime-wry/src/lib.rs) 的 `TAURI_DRAG_RESIZE_BORDERS`)。[官方/源码 + 社区报告]
- **想找回 Snap Layouts 的社区路径**:[Zbrooklyn/tauri-snap-layouts](https://github.com/Zbrooklyn/tauri-snap-layouts)(2026-07 仍更新)用 Win32 子窗口 + HTMAXBUTTON 自写(约 380 行),声称恢复 flyout/圆角/resize/双击最大化/Win+Arrow/Aero Shake/任务栏预览;[#4531 评论区](https://github.com/tauri-apps/tauri/issues/4531)作者强调 "status: upstream does not mean your app is blocked"。**[社区,作者声称,提供可运行模板]**。decorum 插件的替代方案是 enigo 模拟 Win+Z 按键,有弹出位置 bug 且鼠标移开不消失(作者自认,[decorum#29](https://github.com/clearlysid/tauri-plugin-decorum/issues/29),open)[社区]。

### 1.5 标题栏右键系统菜单(还原/移动/大小/最小化/最大化/关闭)

- **Tauri 2 无任何弹出系统菜单的 API**(JS Window 类全量方法已核对,无 ShowSystemMenu 类)[官方/源码核实]。
- **Alt+Space 仍可唤出**:tao 事件循环对 `WM_SYSCHAR` 注释 "Handle system shortcut e.g. Alt+Space for window menu" 并走 `DefWindowProc`——不拦截 [官方/源码]。
- 右键入口物理消失后,社区普遍做法=在 drag region 上监听 contextmenu **自绘**菜单(或 muda)。自绘标题栏与 Tauri Menu 混用有怪异行为(菜单渲染在 DOM 之外、需 Alt 激活):[#12074](https://github.com/tauri-apps/tauri/issues/12074)(open)[单用户报告]。

### 1.6 圆角与阴影

[官方 WindowConfig.shadow 注释](https://tauri.app/reference/config/)(与 [tauri-utils/config.rs](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-utils/src/config.rs) 一致):

- **Windows**:decorated 窗口阴影恒开(关不掉);undecorated + `shadow: true`(默认)→ 1px 白边 + Win11 圆角。实现=tao 的 `WM_NCCALCSIZE` + `MARKER_UNDECORATED_SHADOW`(保留 DWM 画的阴影/圆角,但标题栏连同系统按钮一起消失)。
- **macOS**:undecorated 默认保留系统阴影与圆角(NSWindow `setHasShadow`)。
- **Linux**:"**Unsupported**"——阴影/透明由用户系统的 compositor 决定,tao 维护者原话 "we might not have much control over this at all"([tao#157](https://github.com/tauri-apps/tao/issues/157),closed)[维护者确认]。GNOME undecorated 无阴影无圆角,官方文档零说明、无 workaround。

### 1.7 任务栏缩略图与行为

- 全量检索 tauri 仓库无「无边框导致任务栏缩略图/Aero Peek/点击最小化还原损坏」的 issue(检索穷尽的负面结论);tao 走标准 Win32 任务栏路径(`S_U_TASKBAR_RESTART` 广播恢复 skip-taskbar)。Zbrooklyn 模板也声称 taskbar previews 正常。[负面结论=检索穷尽;正向=社区声称]

### 1.8 系统快捷键

- Win+方向键、Aero Shake:shell 级,仍可用(见 1.4)。Alt+Space:仍可用(见 1.5)。

---

## 二、三条路线在各平台的真实限制

### 2.1 路线①:保留系统标题栏

- **菜单只能放标题栏下一行?**——是,且无变通。官方菜单能力(muda)渲染位置:"as part of the application window for Windows or Linux, or in the menu bar on MacOS"([官方 Window Menu 文档](https://tauri.app/learn/window-menu/));网页内容也一律从系统标题栏下方开始。**WindowConfig 全部字段无任何「内容延伸进标题栏」的配置**(已核对 schema 全量 43 字段)。macOS 的菜单在屏幕顶全局菜单栏,同样不占窗口标题栏行。[官方]
- **暗色主题下系统标题栏颜色能否跟随应用主题?**
  - **Windows:能,但只能黑/白二色**。`window.theme: "Dark"`(或 `App::set_theme` / JS `setTheme`)→ tao 调 `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)` 并重画标题栏([tao/dark_mode.rs](https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/windows/dark_mode.rs)):build>18985 用 attr 20,17763~18985 用 attr 19,<17763 用 SetPropW——**支持下限 Windows 10 1809(17763)**,低于则无效。`theme` 默认值=跟随系统主题。**自定义颜色(DWMWA_CAPTION_COLOR/TEXT_COLOR)未暴露**,需自调 DWM API(如 [spacedrive 的做法](https://github.com/spacedriveapp/spacedrive/pull/3040))。[官方/源码核实]
  - **macOS:能**(10.14+,NSAppearance;theme 为 app-wide)。[官方]
  - **Linux:不能**——`theme` 字段官方明文 "Only implemented on Windows and macOS 10.14+";tao 在 Linux 只读跟随 xdg-desktop-portal color-scheme,`SetTheme` 请求 `unreachable!()`;系统标题栏(SSD)外观由 WM/GTK 主题决定,应用无控制点([官方 config](https://tauri.app/reference/config/) + [tao 源码](https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/linux/event_loop.rs))。[官方]
- **对本项目痛点的映射**:「Windows 刺眼」有官方解(theme 跟随应用暗色即可,本项目 `theme.rs` 已有主题状态,补 `setTheme`/conf 即可);「Linux 刺眼/不统一」无解;「菜单进标题栏行」无解。
- 附:官方对自绘标题栏的总体立场("For macOS, using a custom titlebar will also lose some features provided by the system, such as moving or aligning the window"),并给出折中方案 titleBarStyle: Transparent(见 2.3)。[官方]

### 2.2 路线②:完全自绘三按钮(decorations:false)

- **官方标准做法**([官方 Custom Titlebar 教程](https://tauri.app/learn/window-customization/),四件套):
  1. conf:`"decorations": false`;
  2. capability:`["core:window:default", "core:window:allow-start-dragging"]`(自绘按钮还需补 `allow-minimize` / `allow-toggle-maximize` / `allow-close`,default 集不含这些写操作);
  3. CSS:固定 30px 顶栏(`position: fixed; top:0; left:0; right:0`);
  4. 结构:drag region 放标题栏左侧空白,`.controls` 三按钮独立靠右;JS:`appWindow.minimize()` / `appWindow.toggleMaximize()` / `appWindow.close()`。
  - 官方示例只给了「按钮在右上」一种布局,**没有 macOS 左上/Windows 右上的条件布局官方建议**;对 macOS 官方立场是「自绘会失去系统功能」,替代方案=Transparent titlebar 保留原生红绿灯。[官方]
- **平台差异要点**:
  - `close()` 会先发 closeRequested(可拦截),强关用 `destroy()`。[官方]
  - `toggleFullscreen` **API 不存在**(JS/Rust 均无);macOS 全屏等价物=`setFullscreen(bool)`;另有 `setSimpleFullscreen`(pre-Lion 式不占新 space,原生全屏中返回 false)。macOS 绿色 zoom 按钮官方注释明说"which is also used to enter fullscreen mode"。[官方]
  - `startResizeDragging` macOS 不支持(静默无效,见 1.2)。[官方/源码]
  - 最大化状态同步:`isMaximized()` + `onResized` 监听(CC-Switch 的实现即此,本地参考)。
- **主要代价**(能力清单见第一节):Snap Layouts 悬停丢失(#4531)、右键系统菜单入口消失、Linux 无阴影无圆角、Linux Wayland drag region 问题、需补 7 个左右窗口权限。

### 2.3 路线③:macOS Overlay(titleBarStyle: "Overlay")

- **官方定义与告诫**([官方 config:TitleBarStyle](https://tauri.app/reference/config/)):"Shows the title bar as a transparent overlay over the window's content",三条 caveat:
  1. **标题栏高度随 macOS 版本不同**——控件和标题可能不在你预期的位置;
  2. **必须自定义 drag region**,且「窗口未聚焦时无法拖动」( [#4316](https://github.com/tauri-apps/tauri/issues/4316),open,`acceptsFirstMouse` 不可按元素配置);
  3. 窗口标题颜色跟随系统主题。
- **红绿灯避让:无官方 safe-area 数值**。Tauri 没有 `env(titlebar-area-x)` 之类 CSS 变量([#6030](https://github.com/tauri-apps/tauri/issues/6030),open feat);社区流传的「左侧 ~80px padding」是经验值,非官方。
- **官方正解=移红绿灯而非避让内容**:`trafficLightPosition` 配置(LogicalPosition,**仅创建时,无运行时 setter**;@tauri-apps/api ≥2.4.0;**要求 `titleBarStyle: Overlay` + `decorations: true`**),PR [tauri#12366](https://github.com/tauri-apps/tauri/pull/12366);社区实测 `{"hiddenTitle": true, "titleBarStyle": "Overlay", "trafficLightPosition": {"x": 15, "y": 20}}`([discussion#7978](https://github.com/orgs/tauri-apps/discussions/7978));decorum 默认 inset (12, 16)。本项目窗口由 `from_config` 创建,该字段落在 tauri.conf.json 即生效,单一事实来源不变。[官方 + 社区实测]
- **与 transparent/toolbar 的配合**:
  - `TitleBarStyle::Transparent`(官方推荐的折中):标题栏变透明但仍占位、**保留原生红绿灯与系统拖拽**——官方明说 "This lets you avoid the caveats of using TitleBarStyle::Overlay",配 `hiddenTitle: true` 是「隐标题栏」组合。
  - 窗口级 `transparent: true` 是另一回事,macOS 需 `macos-private-api` feature(**App Store 上架会被拒**,官方 WARNING);decorum 的 `make_transparent()` 声称免 privateApi 实现标题栏透明 [社区声称]。
- **已知坑**:#9503/#7304(Overlay 下拖不动=漏 drag region 属性或权限)、#15623(transparent+overlay 聚焦后反而不能拖,open)、#14253(overlay 下 maximize 按钮渲染成黑色,open)、#13898(双击标题栏后 webview resize 延迟,open)。
- **双层标题栏坑**:Overlay 时系统标题栏是「透明覆盖」而非移除——若网页再画一条带背景色的标题栏且高度超过系统标题栏高度,视觉上就是双层(官方 caveat 1 的直接后果);CC-Switch 的做法是 28px 拖拽条(仅 drag region,无背景块冲突,本地参考)。

### 2.4 CC-Switch 的三平台打法(本地参考,实读源码)

CC-Switch(`D:\AI\GitHub\CC-Switch`)**没有走单一全局方案,而是按平台拆三套**:

| 平台 | 配置 | 前端 | 窗口按钮 |
|---|---|---|---|
| macOS | `titleBarStyle: "Overlay"` + `title: ""`(基础 conf) | 顶部 28px 自绘拖拽条(`DEFAULT_DRAG_BAR_HEIGHT = 28`),内容整体下移 | 原生红绿灯浮在拖拽条上,不自绘 |
| Windows | `tauri.windows.conf.json` 覆写回 `titleBarStyle: "Visible"` + 有标题 | `dragBarHeight = 0`,无自绘 | 系统标题栏 |
| Linux | 基础 conf(titleBarStyle 对 Linux 无效=系统标题栏) | 默认同 Windows;**opt-in 设置** `useAppWindowControls`(默认 false)才切自绘:`getCurrentWindow().setDecorations(false)` + 32px 标题条 | 开启后自绘最小化/toggleMaximize/关闭三按钮(`getCurrentWindow().minimize()/toggleMaximize()/close()` + `onResized`→`isMaximized` 同步) |

关键工程细节(本地参考,均可直接借鉴):

- **Linux 全局禁用 drag region**:`src/lib/platform.ts` 的 `DRAG_REGION_ATTR` 在 Linux 返回空对象(注释:规避 Wayland 下 `gtk_window_begin_move_drag` 窗口事件异常,Tauri #13440);且强调「`data-tauri-drag-region` 是 wry 侧的 attribute 存在性检测,**必须完全不渲染属性才算禁用**,空字符串/"false" 仍会触发」。注意其 App.tsx 顶栏仍硬编码了该属性(与 platform.ts 的禁用策略存在不一致,借鉴时要留意)。
- **CSS 双轨**:`[data-tauri-drag-region] { -webkit-app-region: drag }` + `[data-tauri-no-drag], [data-tauri-drag-region] .no-drag { -webkit-app-region: no-drag }`——用 `.no-drag` 类给拖拽条内的交互子元素开例外(与官方触摸 Tip 同一机制)。
- **capabilities 精确授权**:`core:window:allow-set-skip-taskbar / allow-start-dragging / allow-minimize / allow-toggle-maximize / allow-is-maximized / allow-close / allow-set-decorations`(正是路线②的权限最小集 + 运行时切换装饰)。
- window-state:与本项目**同款 flags**(`POSITION|SIZE|MAXIMIZED`),但它退出路径绕 run loop,故手动 `save_window_state_before_exit`;本项目走 `RunEvent::Exit` 自动落盘,无需手动。

---

## 三、Windows「扩展边框帧」路径(C):无现成实现

诉求=保留系统画的标题栏按钮、同时让网页内容占标题栏区域(DwmExtendFrameIntoClientArea / Electron titleBarOverlay / WCO 一类)。

- **结论:Tauri 生态不存在现成/社区插件实现此路径;官方维护者已表态 Windows 上此路不通(v2)。**
  - 官方维护者 FabianLars([tauri#4973](https://github.com/tauri-apps/tauri/issues/4973),closed):"the problem is that Windows doesn't have similar APIs (to our knowledge at least)";对复刻 Electron titleBarOverlay(WCO):"it is insanely complicated as it draws the controls completely manually...we don't draw anything manually in our libraries...not feasible for us"。[维护者确认]
  - 上游唯一系统级出路 WebView2 Window Controls Overlay 仍是未发布 feature([WebView2Feedback#4532](https://github.com/MicrosoftEdge/WebView2Feedback/issues/4532),tracked)。[官方上游]
  - 用户侧佐证:[tauri#9287](https://github.com/tauri-apps/tauri/issues/9287)(open)开篇 "Because tauri does not support the extension of native windows, I have to manually implement custom windows."
- **重要纠偏:decorum 插件不是这条路**。[clearlysid/tauri-plugin-decorum](https://github.com/clearlysid/tauri-plugin-decorum)(注意:不是 `corteggiano/`,正确仓库在 clearlysid 名下;crates 1.1.1,2024-09,依赖 `tauri = "2.0.0-rc"` 未跟进正式版;GitHub 319 star、自述维护模式)Windows 实现是 `set_decorations(false)` + 注入 HTML 自绘按钮(titlebar.js/controls.js,Segoe Fluent Icons 字体),**不调 DwmExtendFrameIntoClientArea、不保留任何系统按钮**([源码](https://github.com/clearlysid/tauri-plugin-decorum/blob/main/src/lib.rs) 实读);"retain native features like Windows Snap Layout" 靠 enigo 模拟 Win+Z,作者自认有弹出位置 bug([decorum#29](https://github.com/clearlysid/tauri-plugin-decorum/issues/29),open)。macOS 侧提供 `set_traffic_lights_inset`/`make_transparent`;**Linux 是明显短板**(open bug [#51](https://github.com/clearlysid/tauri-plugin-decorum/issues/51)/[#55](https://github.com/clearlysid/tauri-plugin-decorum/issues/55))。[源码核实 + 作者表态]
- **最接近的官方行为**:tao 的 undecorated+shadow 路径(见 1.6)保留了 DWM 阴影/圆角帧,但标题栏连同按钮一起消失——不是本路径。
- **若坚持要:工作量与风险**——只能自写 Win32:child window + HTMAXBUTTON 方案(参考 Zbrooklyn/tauri-snap-layouts,约 380 行,社区可运行模板),要自负 per-monitor DPI、多显示器、与 tao 消息循环共存、随 Windows 更新维护;**无官方支持,风险自担**。或接受「自绘按钮 + 丢失 Snap flyout」(Win+方向键仍可用)。[社区/风险评估]
- 同类其它:window-vibrancy(官方出品)只做毛玻璃/材质效果,不做按钮;tauri-controls(968 star)纯前端按钮组件,Windows 有 open bug 且 Linux 不跟 GTK 主题([#9458 评论区](https://github.com/tauri-apps/tauri/issues/9458))。[社区]

---

## 四、window-state 插件与无边框窗口的兼容注意点(D)

本项目在用 `POSITION|SIZE|MAXIMIZED`(`src-tauri/src/lib.rs`),与 CC-Switch 同款。

- **默认 flags 是全量**(`SIZE|POSITION|MAXIMIZED|VISIBLE|DECORATIONS|FULLSCREEN`,`Default = Self::all()`),其中 **DECORATIONS 会把上次运行的装饰状态恢复回来,覆盖 conf**——社区已提议从默认集移除([plugins-workspace#2617](https://github.com/tauri-apps/plugins-workspace/issues/2617),open)。**本项目显式排除之,天然规避**;若未来走「运行时 setDecorations 切换」(CC-Switch Linux 那种),更不能加回 DECORATIONS,否则设置会被上次状态覆盖。[源码核实 + issue]
- **VISIBLE 同理已被本项目排除**(lib.rs 注释:托盘隐藏/最小化后退出,重启仍正常显示)。[本地]
- **存储用物理像素**(PhysicalSize/PhysicalPosition),跨不同 DPI 显示器按物理值解释;恢复时遍历现存显示器,保存矩形与任一显示器相交才 set_position,否则交 OS 放置(拔显示器不会恢复到不存在的屏幕);最大化前位置单独存 prev_x/prev_y。[源码核实]
- **已知 open bug**(均为单用户报告,数量多且长期存在,提示该插件质量一般,但多与多显示器/DPI/全屏相关,与 decorations 无直接冲突):
  - [#377](https://github.com/tauri-apps/plugins-workspace/issues/377):X11 每次重开 Y 轴 +37px(≈标题栏高度)——**走路线①(系统标题栏)同样中招**,与无边框无关;
  - [#1097](https://github.com/tauri-apps/plugins-workspace/issues/1097)(绝对坐标多显示器)、[#1988](https://github.com/tauri-apps/plugins-workspace/issues/1988)(双屏全屏恢复错分辨率)、[#3521](https://github.com/tauri-apps/plugins-workspace/issues/3521)(macOS scale_factor=1.0 位置翻倍)、[#3289](https://github.com/tauri-apps/plugins-workspace/issues/3289)(macOS 随机恢复成小窗)、[#3215](https://github.com/tauri-apps/plugins-workspace/issues/3215)(退出全屏不复原)、[#2733](https://github.com/tauri-apps/plugins-workspace/issues/2733)(每次重开缩一个菜单高度)。
- 未发现「window-state × decorations:false」的专属损坏报告;两者组合无已知不兼容。[检索穷尽的负面结论]

---

## 五、对「拍板」的事实输入(只摆事实,不替决策)

1. **「菜单与系统窗口按钮同行」**:仅 macOS Overlay 原生可行(且用 `trafficLightPosition` 可把红绿灯移进自布局);Windows/Linux 想同行 ⇒ 只能全自绘(路线②),Windows 附带丢 Snap Layouts 悬停(tauri#4531,上游阻塞,短期内无解),Linux 附带无阴影无圆角 + Wayland drag region 风险。
2. **「Windows 标题栏刺眼」**:路线① 即可解——`theme` 跟随应用主题(DWMWA_USE_IMMERSIVE_DARK_MODE,Win10 1809+,仅黑/白);本项目 `theme.rs` 已有主题状态,接入成本低。Linux 刺眼无解(GTK 主题决定);macOS 10.14+ 可跟随。
3. **折中先例(本地参考)**:CC-Switch 的「macOS Overlay + Windows 系统标题栏 + Linux 默认系统标题栏(自绘仅 opt-in)」= 每平台取「痛点最小」的一侧,而非三平台强行统一;其 platform.ts/index.css/capabilities 的落地细节可直接复用。
4. **若走路线②**:Tauri ≥2.11 的 `data-tauri-drag-region="deep"` 消掉子元素痛点;权限最小集照抄 CC-Switch 的 7 项;Linux 侧要明确 Wayland/X11 的测试范围(CC-Switch 用「Linux 禁 drag region + 系统标题栏兜底」回避)。
5. **路线④(扩展边框帧)可以直接排除**:无官方路径、无可靠社区插件(decorum 名不副实)、WebView2 WCO 未发布;仅剩「自写 Win32 子窗口」这一高风险选项。

---

## 附:来源索引

**官方文档**

- 窗口定制指南(decorations/drag region/权限/官方 titlebar 教程):https://tauri.app/learn/window-customization/
- WindowConfig 参考(shadow/theme/titleBarStyle/trafficLightPosition/transparent):https://tauri.app/reference/config/
- JS window API:https://tauri.app/reference/javascript/api/namespacewindow/
- 菜单(平台渲染位置):https://tauri.app/learn/window-menu/
- window-state 插件:https://tauri.app/plugin/window-state/
- Capabilities:https://tauri.app/security/capabilities/

**官方源码**

- drag 脚本(双击最大化/可点击拦截/deep):https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/src/window/scripts/drag.js
- drag 示例(bare/deep/false):https://github.com/tauri-apps/tauri/tree/dev/examples/drag
- 窗口权限全量清单:https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/permissions/window/autogenerated/reference.md
- tao Windows 暗色标题栏(DWMWA_USE_IMMERSIVE_DARK_MODE,attr 19/20 分界 18985):https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/windows/dark_mode.rs
- tao macOS drag_resize_window NotSupported:https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/macos/window.rs
- tao Linux SetTheme unreachable:https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/linux/event_loop.rs
- window-state 源码(StateFlags 默认全量/物理像素/显示器相交检查):https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/window-state/src/lib.rs

**关键 issue/PR(标注状态)**

- tauri#4531(open)Snap Layouts 上游阻塞:https://github.com/tauri-apps/tauri/issues/4531
- tauri#4973(closed,维护者表态)Windows 无扩展边框 API:https://github.com/tauri-apps/tauri/issues/4973
- tauri#4316(open)Overlay 未聚焦不可拖:https://github.com/tauri-apps/tauri/issues/4316
- tauri#15062(merged,v2.11 drag region "deep"):https://github.com/tauri-apps/tauri/pull/15062
- tauri#12366(merged,trafficLightPosition):https://github.com/tauri-apps/tauri/pull/12366
- tauri#6030(open)无 env() safe-area 变量:https://github.com/tauri-apps/tauri/issues/6030
- tauri#13440(CC-Switch 注释引用的 Wayland drag 问题,本地参考)
- tao#1218(merged 2026-06)Wayland CSD 回退 GTK 默认:https://github.com/tauri-apps/tao/pull/1218
- plugins-workspace#2617(open)DECORATIONS 不该默认恢复:https://github.com/tauri-apps/plugins-workspace/issues/2617
- plugins-workspace#377(open)X11 Y 轴漂移:https://github.com/tauri-apps/plugins-workspace/issues/377
- WebView2Feedback#4532(tracked)WCO feature:https://github.com/MicrosoftEdge/WebView2Feedback/issues/4532

**社区(作者声称,未经官方背书)**

- tauri-plugin-decorum:https://github.com/clearlysid/tauri-plugin-decorum (含 #29 Snap 弹出 bug、#51/#55 Linux 问题)
- Zbrooklyn/tauri-snap-layouts(Win32 子窗口恢复 Snap Layouts 模板):https://github.com/Zbrooklyn/tauri-snap-layouts
- discussion#7978(trafficLightPosition 实测):https://github.com/orgs/tauri-apps/discussions/7978
- window-vibrancy(仅材质效果):https://github.com/tauri-apps/window-vibrancy
- tauri-controls:https://github.com/agmmnn/tauri-controls

**本地参考**

- CC-Switch(`D:\AI\GitHub\CC-Switch`):`src-tauri/tauri.conf.json` 与 `tauri.windows.conf.json`(平台拆分配置)、`src/App.tsx`(自绘三按钮/setDecorations/onResized 同步)、`src/lib/platform.ts`(Linux 禁 drag region)、`src/index.css`(-webkit-app-region/no-drag)、`src-tauri/capabilities/default.json`(7 项窗口权限)、`src-tauri/src/lib.rs`(window-state flags/手动 save)
