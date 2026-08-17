# 实测：dsh 能否被 iframe 嵌入（wayfinder #35）

- 日期：2026-08-17
- 调研目的：#26（[overlay-feasibility](https://github.com/Buktal/deepseek-desktop/blob/research/overlay-feasibility/docs/research/overlay-feasibility.md)）判多 webview 叠层三平台 No-Go 后，iframe 是新主线。本票实测其硬前提——dsh Web UI 可否被 iframe 嵌入（响应头 + 真实渲染）——并查证壳的导航拦截对 iframe 的生效范围、boot 流水线的端口获取与 src 跟随时机，供「拍板：叠层架构技术路径」定选项集。
- 方法：本机实测（curl 原始响应头 + headless Chromium 实嵌渲染）+ 一手来源查证（Microsoft WebView2 文档、wry/tauri 源码与 issues）+ 本项目源码精读（`src-tauri/src/dsh.rs` / `navigation.rs` / `upgrade.rs`）。除标注外全部结论有实测输出或来源链接。
- 实测环境：Windows 11；dsh `0.1.0-rc.6`（npm 全局 `E:\Nvm\nodejs\node_modules\@deepseek-ai\dsh`）；本项目锁定 tauri 2.11.5 / wry 0.55.1；渲染用 Edge headless（Chromium，WebView2 同引擎，帧安全行为同源）。

---

## 0. 结论：Go（iframe 主线硬前提全部成立，实测通过）

| # | 问题 | 结论 | 依据 |
|---|---|---|---|
| 1 | dsh 响应头有无 `X-Frame-Options` / `Content-Security-Policy` | **无**（主页面、SPA 任意路由、全部静态资源、插件 client.js 全查） | §1 实测原始响应头 |
| 2 | 真实 Chromium 实嵌能否渲染 | **能**：dsh Web UI 在 iframe 内完整渲染（首启 API Key 引导弹窗），load 事件触发，跨源隔离符合预期 | §2 实测截图 |
| 3 | 壳 `on_navigation` / `on_new_window` 对 iframe 内导航 / iframe 内 `target=_blank` 是否生效 | **Windows 上两者都不生效**（wry 只订阅顶层 `NavigationStarting`，未接 `FrameNavigationStarting`；且 wry#1593 实测：Windows 上 iframe 内发起的导航与新窗口请求两个回调都不触发）。各 OS 行为还互相分叉（macOS 新窗口回调触发 / Linux 导航回调触发）。处置：iframe 内拦截须移到页面层（全帧初始化脚本或 dsh 服务自身），见 §3 | §3 一手来源 |
| 4 | boot 流水线如何拿就绪信号与动态端口；iframe src 跟随时机 | 就绪信号 = stdout 行 `dsh web: http://127.0.0.1:<port>`（`--port 0` 动态分配）；src 需跟随的时点共 4 个（boot 就绪 / 升级链就绪 /「稍后/返回」重启 / 崩溃后重试），单一事实来源 `record_dsh_url` 已存在，改为「推 URL 给壳页」即可 | §4 源码行号 |

**对「拍板」的意义**：#26 推荐的备选 B（单 webview + iframe 常驻壳）最后一条「待实测定案」的硬前提（响应头放行）已实测通过，且渲染层面真实可渲染——iframe 路径技术上 Go，可进入实现评估。

---

## 1. 实测：dsh web 起服务与响应头

### 1.1 起服务（复刻 boot 流水线语义）

boot 流水线的真实命令（`src-tauri/src/dsh.rs` `spawn_dsh`，L739-744）：`node <bin.js> web --port 0`。本票按同一命令起服务：

```bash
$ DSH_TELEMETRY_DISABLED=1 node "E:/Nvm/nodejs/node_modules/@deepseek-ai/dsh/lib/bin.js" web --port 0
# stdout 就绪行（约 8s 内出现，profile 已存在；首启约 65s，见 dsh.rs L59 注释）：
dsh web: http://127.0.0.1:57130
```

端口 57130 = `--port 0` 由 OS 动态分配；就绪行与 `dsh.rs` 的 `READY_PREFIX`（L57）/`parse_ready_line`（L778）完全匹配。

### 1.2 主页面响应头（原样）

```bash
$ curl -sI http://127.0.0.1:57130/
HTTP/1.1 200 OK
content-type: text/html; charset=utf-8
Date: Mon, 17 Aug 2026 11:35:14 GMT
Connection: keep-alive
Keep-Alive: timeout=5
```

**无 `X-Frame-Options`、无 `Content-Security-Policy`（连通用 CSP 都没有，不止缺 `frame-ancestors`）。**

### 1.3 SPA 任意路由（iframe 内文档导航的目标也放行）

dsh 是 SPA，任意路径回同一 index.html；iframe 内的文档级导航同样受帧头约束，实测：

```bash
$ curl -sI http://127.0.0.1:57130/conversation/test-route-xyz
HTTP/1.1 200 OK
content-type: text/html; charset=utf-8
Date: Mon, 17 Aug 2026 11:35:42 GMT
Connection: keep-alive
Keep-Alive: timeout=5
```

### 1.4 主页面全部静态资源（原样）

主页面引用的资源（`/assets/*`、`/favicon.svg`、`/manifest.webmanifest`）与两个代表性插件脚本逐个 HEAD，全部一致：

```bash
$ for p in /assets/index-CSGf6Qzd.css /assets/vendor-Cjbwl5VI.js /assets/vendor-CjyC-hUb.css \
           /assets/index-Dqw48FrP.js /favicon.svg /manifest.webmanifest \
           "/plugins/@deepseek-ai/dsh-client-runtime/client.js?rev=5404bd0408a5" \
           "/plugins/@deepseek-ai/dsh-client-ui-layout/client.js?rev=5ab8c01f4dbb"; do
    echo "=== $p"; curl -sI "http://127.0.0.1:57130$p"; done

=== /assets/index-CSGf6Qzd.css
HTTP/1.1 200 OK
content-type: text/css; charset=utf-8
Date: Mon, 17 Aug 2026 11:35:27 GMT
Connection: keep-alive
Keep-Alive: timeout=5
=== /assets/vendor-Cjbwl5VI.js
HTTP/1.1 200 OK
content-type: text/javascript; charset=utf-8
Date: Mon, 17 Aug 2026 11:35:27 GMT
Connection: keep-alive
Keep-Alive: timeout=5
=== /assets/vendor-CjyC-hUb.css
（同上，略：content-type: text/css; charset=utf-8）
=== /assets/index-Dqw48FrP.js
（同上，略：content-type: text/javascript; charset=utf-8）
=== /favicon.svg
（同上，略：content-type: image/svg+xml）
=== /manifest.webmanifest
（同上，略：content-type: application/manifest+json）
=== /plugins/@deepseek-ai/dsh-client-runtime/client.js?rev=5404bd0408a5
HTTP/1.1 200 OK
content-type: text/javascript; charset=utf-8
cache-control: no-cache
Date: Mon, 17 Aug 2026 11:35:21 GMT
Connection: keep-alive
Keep-Alive: timeout=5
```

判定依据：Chromium（含 WebView2）对 http(s) 文档默认允许被嵌入，仅当响应带 `X-Frame-Options: DENY/SAMEORIGIN` 或 CSP `frame-ancestors` 不含父源时才拒绝。dsh 所有文档响应两者皆无 → 放行。§2 用真实 Chromium 验证了这一判定。

---

## 2. 实测：iframe 实嵌渲染（真实 Chromium）

### 2.1 测试页与命令

宿主页 `iframe-test.html`（跨源关系与生产一致：父 `http://127.0.0.1:8765`，子 `http://127.0.0.1:57130`，不同端口即不同源；生产为父 `http://tauri.localhost` / dev `http://localhost:1420`，子同为 dsh 动态端口，同为跨源 http→http，无混合内容问题）：

```html
<div id="status">parent loaded; iframe pending…</div>
<iframe id="f" src="http://127.0.0.1:57130/"></iframe>
<script>
  f.addEventListener('load', () => {
    s.textContent = 'iframe LOAD event fired at Nms';
    try { s.textContent += ' | same-origin?! ' + f.contentWindow.location.href; }
    catch (e) { s.textContent += ' | cross-origin (location read blocked: ' + e.name + ')'; }
  });
</script>
```

```bash
$ python -m http.server 8765 --bind 127.0.0.1   # 宿主静态服务
$ "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
    --headless=new --disable-gpu --window-size=1400,900 \
    --screenshot=iframe-shot.png --timeout=20000 \
    "http://127.0.0.1:8765/iframe-test.html"
48190 bytes written to file E:/Temp/dsh-iframe-test/iframe-shot.png
```

### 2.2 结果（截图 `iframe-shot.png`，与本文件同目录提交）

- **iframe 内是完整渲染的 dsh Web UI**：深色主题 + 首启引导弹窗「添加一个 API Key 开始使用 / 配置 DeepSeek 官方模型，即可开始使用」，含 API 密钥输入框与「稍后配置 / 保存并继续」按钮——dsh 的 100+ 插件客户端脚本已在 iframe 内正常执行（引导弹窗是插件渲染的），不是空白页、不是 "refused to connect" 错误页。
- 状态栏（宿主页 JS 写入）：**`iframe LOAD event fired | cross-origin (location read blocked: SecurityError)`**——iframe 文档已提交加载，且父读子 `location` 抛 `SecurityError`，跨源隔离与整窗 remote 模式（#26 §6）一致。

> 引擎一致性说明：WebView2 与 Edge 同为 Chromium，帧安全头（XFO/frame-ancestors）与混合内容的强制行为相同；本节直接在 Chromium 内核实测渲染成功，无需再单独以 WebView2 复测。未实测项：WebView2 控件内的实际呈现（属实现阶段验证，非本票前提）。

### 2.3 用后清理

dsh（PID 2292）与静态服务（PID 36956）均 `taskkill /PID <pid> /T /F` 树杀并经 netstat 复核端口已释放。机器上另有 4 个 `dsh web` 进程为本票开始前已存在（08-14 两个孤儿、19:01 一个、`deepseek-desktop.exe` 的一个），非本票所起，未动。

---

## 3. 事实查证：`on_navigation` / `on_new_window` 对 iframe 是否生效

本项目现状（`src-tauri/src/navigation.rs` L116 `create_main_window`）：主窗口经 `WebviewWindowBuilder::from_config` 挂两个回调——`on_navigation`（顶层导航放行判定 + 外链交系统浏览器）、`on_new_window`（一律交系统浏览器后 `Deny`）。改 iframe 后，dsh 的页面行为发生在 iframe 里，两个回调的覆盖范围如下。

### 3.1 `on_navigation`：Windows 上不覆盖 iframe 内部导航（Linux 反而触发，见 §3.3 分叉）

- WebView2 事件语义（[Microsoft 官方文档 ICoreWebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2)）：`NavigationStarting` **只在 main frame 请求导航时触发**（"NavigationStarting runs when the WebView **main frame** is requesting permission to navigate to a different URI"）；子 frame 走独立事件 `FrameNavigationStarting`（"triggers when a **child frame** in the WebView requests permission to navigate"）。
- wry 0.55.1 源码（[src/webview2/mod.rs L674-690](https://github.com/tauri-apps/wry/blob/wry-v0.55.1/src/webview2/mod.rs)）只 `add_NavigationStarting` 转发给 `on_navigation`（返回 false → `SetCancel`）；**全文件无 `FrameNavigationStarting` / `CoreWebView2Frame` 订阅**（当前 dev 分支同样只有 `add_NavigationStarting` 与 `add_NewWindowRequested`）。
- 实测佐证（[wry#1593](https://github.com/tauri-apps/wry/issues/1593)，Windows 11，2025-08，open、`[type: bug]`、零评论、无关联 PR）：iframe 内的 src 加载、`location` 跳转、锚点导航均不进 navigation handler。
- Tauri 文档（[docs.rs WebviewWindowBuilder](https://docs.rs/tauri/2.11.5/tauri/webview/struct.WebviewWindowBuilder.html)）对两个回调只述「when the webview navigates」/「window.open API」，未提 frame 范围——无背书也无豁免。
- 社区现状：wry/tauri 均无 frame 级导航控制 API；`gh search issues "FrameNavigationStarting"` 在 tauri-apps 两仓零结果——连 feature request 都还没有。

**含义**：iframe 模式下 `on_navigation` 只管壳页自身的顶层导航（仍需要——壳页常驻后防它被整窗导航走，`should_allow_navigation` 的壳本地页放行分支保留）。Windows 上 dsh 页内的导航（含 SPA 路由）不经壳判定。

### 3.2 `on_new_window`：iframe 内发起的新窗口请求，Windows 上**同样不触发**

- 文档层面：`NewWindowRequested`（"runs when content inside the WebView requests to open a new window"）未写明顶层限定，事件参数（Uri/WindowFeatures/IsUserInitiated）与 frame 无关（[ICoreWebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2)）——按文档语义似应覆盖 iframe 发起。
- **实测推翻（对 iframe 场景）**：[wry#1593](https://github.com/tauri-apps/wry/issues/1593) 原话 "**On Windows, navigating inside an iframe doesn't activate the navigation handler or the new window request handler**"——Windows 上 iframe 内的 `window.open` / `target=_blank` 绕过 wry 的 `on_new_window`，落到 WebView2 默认行为（弹出 WebView2 自管的 popup 窗口；确切形态本票未实测，实现期需验证）。相关原生报告 [WebView2Feedback#108](https://github.com/MicrosoftEdge/WebView2Feedback/issues/108)。
- wry 0.55.1 对 `add_NewWindowRequested` 无条件注册、`Deny` → `SetHandled(true)`（`WindowProxy` 为空对象、无页面加载）——这只对**事件触发得到**的路径有效（顶层 `window.open` / 整窗模式现状），iframe 场景在 Windows 上触发不到。

### 3.3 三平台分叉与社区处置（#1593 实测矩阵）

| 行为 | Windows (WebView2) | macOS (WKWebView) | Linux (WebKitGTK) |
|---|---|---|---|
| iframe 内导航 → `on_navigation` | 不触发 | 不触发 | **触发** |
| iframe 内新窗口 → `on_new_window` | **不触发** | **触发** | 不触发 |

三平台行为互不相同且 #1593 仍 open——**Rust 侧拦截对 iframe 不可作为跨平台依靠**。已知处置方向（实现期评估，非本票定案）：
1. **页面层拦截（可移植，推荐评估）**：壳经 Tauri `initialization_script_for_all_frames`（文档明示 "runs on all frames (main frame and also sub frames)"，Windows 注明 "scripts are always added to subframes"）向 iframe 注入链接拦截脚本，命中外链后 `window.parent.postMessage` 通知壳页，由壳（本地 origin、有 Tauri API）交 `tauri-plugin-opener` 开系统浏览器——跨源 iframe 无法直接碰 Tauri API，但 postMessage 允许。
2. dsh 服务自身处置外链（上游配合，耦合深）。
3. fork/补 wry 接 `add_FrameNavigationStarting`（`ICoreWebView2` 上现成事件，Windows-only 能力，跨平台仍需另做）。

### 3.4 帧安全与混合内容（对 §1/§2 结论的文档佐证）

- WebView2 即 Chromium，对 http(s) 文档强制 `X-Frame-Options` / CSP `frame-ancestors`（[MDN X-Frame-Options](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/X-Frame-Options)：`DENY`/`SAMEORIGIN` 与 origin 无关一律生效，非 HTTPS 限定）；`127.0.0.1:<port>` ≠ `tauri.localhost`（跨源）→ dsh 若发 XFO/SAMEORIGIN 即被拒。§1 实测两者皆无 → 放行，§2 实嵌渲染验证了这一判定。
- 混合内容不成立：父壳页是 http（tauri 默认 `http://<scheme>.localhost`，`use_https_scheme` 默认 false，[tauri_utils WindowConfig](https://docs.rs/tauri-utils/latest/tauri_utils/config/struct.WindowConfig.html)），http→http 无混合内容；即便未来切 https，`127.0.0.0/8` / `localhost` / `*.localhost` 属 [W3C Secure Contexts](https://www.w3.org/TR/secure-contexts/#is-origin-trustworthy) "potentially trustworthy"，仍不触发。
- Tauri ACL 对 iframe 的边界与整窗 remote 一致（#26 §6：零 remote capability 即维持现状；Linux/Android 无法区分 iframe 与窗口本身的请求，零 remote 下无实际影响）。

---

## 4. boot 流水线：就绪信号、动态端口与 iframe src 时机点（源码精读）

### 4.1 就绪信号与端口获取（现状，`src-tauri/src/dsh.rs`）

- **起服务**：`spawn_dsh`（L735-773）执行 `node <bin.js> web --port 0`（bin 路径运行时经 `npm root -g` 解析），stdout/stderr 各起读线程送 channel。
- **就绪信号**：`wait_ready`（L813-849）逐行收 stdout，`parse_ready_line`（L778-791）匹配前缀 `dsh web: http://` 并取端口——**stdout 打印该行即服务就绪**（dsh 源码注释明确 readiness signal，dsh.rs L56）。动态端口就取自这一行（`--port 0` → OS 分配，实测 57130）。
- **兜底确认**：`tcp_wait`（L806-808）轮询 TCP 连通 30 × 500ms，零 HTTP 依赖。
- **就绪后**：`dsh_url_for_port(port)` 拼出 `http://127.0.0.1:{port}`（L527-529，单一事实来源）→ `record_dsh_url` 存入 `DshManager.dsh_url`（L232-236）→ 等前端动画信号（≤1500ms）→ `navigate_main_window` 整窗导航过去（L925-939）。

### 4.2 iframe src 需要跟随端口变化的时机点（共 4 个）

iframe 模式下壳页常驻，不再整窗导航；「端口变了 → iframe.src 要换」的全部时点：

| 时点 | 代码位置 | 现状动作 | iframe 模式动作 |
|---|---|---|---|
| ① boot 就绪 | `dsh.rs` L911-939 | `record_dsh_url` + 整窗导航 | 推 URL 给壳页，壳页 set `iframe.src` |
| ② 升级链就绪（杀旧起新，**端口必变**） | `upgrade.rs` L717-751 | 同上（`record_dsh_url` + 导航） | 同 ① |
| ③ 升级卡「稍后/返回」且 dsh 已死 | `upgrade.rs` L422-458 `dismiss` | 重起 dsh → `wait_ready` → `record_dsh_url` + 导航 | 同 ① |
| ④ dsh 意外退出（reaper 弹窗）后重试 | `dsh.rs` L951-983 reaper；重试走 `boot_start`（phase Error → 新流水线新端口） | 新一轮 boot → 新端口导航 | 同 ① |

**实现接缝**：`DshManager.dsh_url`（`record_dsh_url`）已是「当前 dsh URL」的单一事实来源，四个时点都已写入它——iframe 模式只需把「读它然后 `navigate_main_window`」换成「把它推给壳页」（现有 `boot-state` 事件通道加 URL 字段，或新事件）；顺带 `navigate_to_dsh` 命令与 `wait_navigate_signal_or_timeout` 动画等待（L363-372，为「整窗导航走人」设计）可整体退役。①②③④ 每次都是 `--port 0` 新动态端口，**绝不缓存旧端口复用**。

---

## 5. 风险与观察项

1. **上游依赖风险（#26 已提，本票证实现状）**：dsh 当前不设 XFO/CSP；将来上游若加 `frame-ancestors`/XFO，iframe 主线即死。属可接受的上游耦合，升级回归时值得加一条响应头检查。
2. **iframe 内外链/新窗口不再被 Rust 侧拦截**（§3，Windows 上两回调对 iframe 均不触发，三平台行为还分叉）：整窗模式「外链交系统浏览器」的保护在 iframe 模式下失效，须移到页面层（§3.3 选项 1，全帧初始化脚本 + postMessage）。属实现项，非 Go/No-Go 阻断——但拍板时应把它计入 iframe 路径的工作量。
3. **本机孤儿 dsh 进程**（顺带观察）：实测机上存在 08-14 遗留的两个 `dsh web` 孤儿进程（父进程已退）——属 #22「Io 路径孤儿进程修复」合入前的历史遗留，非本票范围，仅供 boot/退出收敛稳定性参考。
4. 本票未实测：WebView2 控件内实际呈现、iframe 内键盘快捷键/剪贴板/全屏、dsh 路由跳转对 iframe history 的影响（#26 §5.2 已列，属实现评估票）。

## 来源索引

- 实测（本票，2026-08-17）：curl 原始响应头（§1）、Edge headless 渲染截图 `iframe-shot.png`（§2）
- 本项目源码：`src-tauri/src/dsh.rs`（L56-57/L357-372/L527-529/L735-791/L813-849/L911-939）、`src-tauri/src/navigation.rs`（L87-110/L116-152）、`src-tauri/src/upgrade.rs`（L422-458/L717-751）
- Microsoft WebView2 文档（`NavigationStarting` main-frame 语义、`FrameNavigationStarting` child-frame 语义、`NewWindowRequested` 与 `put_Handled`）：https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2
- wry 0.55.1 源码（只 `add_NavigationStarting` / `add_NewWindowRequested`，无任何 frame 级订阅）：https://github.com/tauri-apps/wry/blob/wry-v0.55.1/src/webview2/mod.rs#L674-L690
- wry#1593（iframe 触发两回调的三平台分叉实测，open、type:bug、无关联 PR）：https://github.com/tauri-apps/wry/issues/1593
- WebView2Feedback#108（NewWindowRequested 覆盖范围的原生层报告）：https://github.com/MicrosoftEdge/WebView2Feedback/issues/108
- Tauri v2 `WebviewWindowBuilder.on_navigation` / `on_new_window` / `initialization_script_for_all_frames`（2.11.5）：https://docs.rs/tauri/2.11.5/tauri/webview/struct.WebviewWindowBuilder.html
- tauri_utils `WindowConfig`（`use_https_scheme` 默认 false → 壳页为 http origin）：https://docs.rs/tauri-utils/latest/tauri_utils/config/struct.WindowConfig.html
- MDN X-Frame-Options（DENY/SAMEORIGIN 语义，非 HTTPS 限定）：https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/X-Frame-Options ；MDN Mixed content（http→http 非混合内容）：https://developer.mozilla.org/en-US/docs/Web/Security/Mixed_content
- W3C Secure Contexts §3.1（127.0.0.0/8 / localhost / *.localhost 为 potentially trustworthy）：https://www.w3.org/TR/secure-contexts/#is-origin-trustworthy
- 前序调查（#26，多 webview 叠层 No-Go + iframe 备选 B）：`research/overlay-feasibility` 分支 `docs/research/overlay-feasibility.md`
