//! 系统托盘(定稿结构,见 #9):显示/隐藏窗口、主题、开机自启、设置▸关闭行为、
//! 检查更新、退出。
//!
//! M3 架构(#33 拍板 / #38 施工):菜单模型与动作全在 Rust——`menu::build_snapshot`
//! (纯函数:状态源 + locale → MenuSnapshot)是唯一构建处,本模块是托盘这一个
//! 薄投影层(muda 投影),另一投影是 `menu-state` 事件 + `menu_snapshot` 命令
//! (前端 DropdownMenu 纯映射渲染);菜单结构与快照字段见 menu.rs 模块文档。
//! 每次状态变化 `refresh_menu` 重建托盘 + emit 新快照,两处入口同一状态。
//!
//! 功能分发(`dispatch_menu_action`,托盘 on_menu_event 与前端 menu_action
//! 命令共用同一张 id → 动作表):
//! - 主题:点击主题项 → `theme::choose`(theme.rs 是主题的单一事实源:
//!   更新内存、持久化、同步原生窗口、推 `theme-changed` 生效主题事件给 boot UI)。
//!   勾选状态以 theme.rs 内存为事实源;本处仍按 #9 契约推 `tray-theme` 事件
//!   (payload 为 "light"|"dark"|"system" 选择串),boot UI 实际消费的是
//!   `theme-changed`("light"|"dark" 生效主题,见 theme.rs 模块文档)。
//!   注意:事件只到 boot UI(dsh 页是 remote origin,ACL 拒绝,见 dsh.rs 安全语义),
//!   与红线一致——dsh 页面不碰主题。
//! - 关闭行为(#38):菜单「设置▸关闭行为」三选 → `close::set`(close.rs 是
//!   关闭行为的单一事实源:更新内存、持久化);close handler 读之(见 lib.rs)。
//! - 检查更新(#3 事件契约变更):原占位「推 `tray-check-update` 事件给前端」被取代——
//!   检查逻辑全在 Rust 侧(update.rs / upgrade.rs),托盘点击直接调用检查模块,
//!   前端不再监听;事件 emit 移除(不留死契约)。
//! - 升级通知形态(#3 §1,两层升级共用同一 Rust 侧机制):自动检测发现新版 →
//!   徽标图标变体 + 动态菜单项(app「升级到 vX」/ dsh「升级 dsh 到 vX」)+ tooltip,
//!   不弹窗打断;点击动态菜单项 → 显示窗口 + 推卡片请求事件(upgrade-card-request
//!   / update-card-request),前端按状态渲染对应升级卡片浮层(壳页常驻,无整窗
//!   导航,#36;自动检测只亮徽标不弹卡片,#3 §1)。
//! - 手动检查入口(#17 组合编排 on_check_update,原生对话框已退役 #39):
//!   dsh 层先答(dsh 新版 → shell-dialog AlertDialog;检查失败 → toast),
//!   应用层兜底(应用新版 → AlertDialog 附 notes;无新版 → toast 合并
//!   「已是最新」附 dsh 版本);任一升级流水线在途 → toast「升级正在进行中」
//!   (#31 行为修正,取代静默 no-op),菜单快照同步把「检查更新」置为 disabled。
//! - 弹窗回答(#31):前端 `shell_dialog_respond` → dispatch_dialog_response
//!   (与菜单动作同一张动作表;关闭三选的执行与「记住勾选」持久化在此分发)。
//! - 左键单击托盘图标:窗口可见且已聚焦时隐藏,否则显示并聚焦——纯 toggle 的陷阱是
//!   窗口被其它窗口挡住时,用户本想"唤出"结果却把窗口藏了。
//! - 退出:先杀 dsh 子进程再 exit(所有退出路径最终经 RunEvent::ExitRequested 再杀一次,
//!   kill_child 幂等,无副作用)。

use std::sync::Mutex;
use std::thread;

use tauri::menu::{
    CheckMenuItem, Menu, MenuBuilder, MenuItem as MudaMenuItem, SubmenuBuilder,
};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::close::CloseBehavior;
use crate::menu::{MenuItem as SnapshotMenuItem, MenuItemKind, MenuSnapshot};
use crate::{autostart, close, dialog, dsh, locales, menu, theme, update, upgrade};
use crate::theme::ThemeChoice;

/// 托盘图标句柄(发现新版时换徽标变体 / 恢复,见 set_app_update/set_dsh_update)。
static TRAY: Mutex<Option<TrayIcon<Wry>>> = Mutex::new(None);
/// 升级通知槽位(单一事实源,set_app_update/set_dsh_update 维护):
/// 存「待升级版本」(非标签文案——语言在 menu::build_snapshot 时才判定拼标签),
/// 任一非空 → 徽标图标变体;动态菜单项按优先级插入(先 dsh 后应用)。
static APP_UPDATE_VERSION: Mutex<Option<String>> = Mutex::new(None);
static DSH_UPDATE_VERSION: Mutex<Option<String>> = Mutex::new(None);

/// 菜单事件 id → 主题选择。纯函数,可测;未知 id 返回 None。
/// 主题状态与映射的单一事实源在 theme.rs(ThemeChoice::menu_id ↔ from_payload)。
/// 生产路径唯一调用方是 `dispatch_menu_action`(#33:并入统一分发函数)。
fn theme_choice_from_id(id: &str) -> Option<ThemeChoice> {
    match id {
        "theme-light" => Some(ThemeChoice::Light),
        "theme-dark" => Some(ThemeChoice::Dark),
        "theme-system" => Some(ThemeChoice::System),
        _ => None,
    }
}

/// 菜单事件 id → 关闭行为。纯函数,可测;未知 id 返回 None。
/// 映射的单一事实源在 close.rs(CloseBehavior::menu_id ↔ payload)。
fn close_behavior_from_id(id: &str) -> Option<CloseBehavior> {
    match id {
        "close-ask" => Some(CloseBehavior::Ask),
        "close-minimize" => Some(CloseBehavior::Minimize),
        "close-quit" => Some(CloseBehavior::Quit),
        _ => None,
    }
}

/// muda 投影:菜单快照 → 托盘原生菜单(薄层,不承载任何状态判断;
/// 结构与文案全部来自快照,#33)。子菜单递归投影(设置▸关闭行为两层嵌套)。
fn build_menu(app: &AppHandle, snapshot: &MenuSnapshot) -> tauri::Result<Menu<Wry>> {
    let mut builder = MenuBuilder::new(app);
    for item in &snapshot.items {
        builder = append_snapshot_item(builder, app, item)?;
    }
    builder.build()
}

/// 快照项 → muda 控件(MenuBuilder 与 SubmenuBuilder 方法集相同,经统一
/// trait 收敛,避免同一 kind 分发写两份——两份实现迟早漂移)。
trait MenuAppend {
    /// 追加一个已构建的 muda 控件(MenuItem / CheckMenuItem / Submenu)。
    fn push(self, item: &dyn tauri::menu::IsMenuItem<Wry>) -> Self;
    /// 追加分隔线。
    fn push_separator(self) -> Self;
}

impl<'a> MenuAppend for MenuBuilder<'a, Wry, AppHandle<Wry>> {
    fn push(self, item: &dyn tauri::menu::IsMenuItem<Wry>) -> Self {
        self.item(item)
    }
    fn push_separator(self) -> Self {
        self.separator()
    }
}

impl<'a> MenuAppend for SubmenuBuilder<'a, Wry, AppHandle<Wry>> {
    fn push(self, item: &dyn tauri::menu::IsMenuItem<Wry>) -> Self {
        self.item(item)
    }
    fn push_separator(self) -> Self {
        self.separator()
    }
}

/// 单层投影:按 kind 分发(disabled → muda enabled 参数,勾选 → CheckMenuItem
/// 固定勾选态,重建时随快照回到内存事实源);子菜单递归投影。
fn append_snapshot_item<B: MenuAppend>(
    builder: B,
    app: &AppHandle,
    item: &SnapshotMenuItem,
) -> tauri::Result<B> {
    match item.kind {
        MenuItemKind::Separator => Ok(builder.push_separator()),
        MenuItemKind::Submenu => {
            let mut sub = SubmenuBuilder::with_id(app, item.id.as_str(), item.label.as_str());
            for child in item.children.as_deref().unwrap_or(&[]) {
                sub = append_snapshot_item(sub, app, child)?;
            }
            Ok(builder.push(&sub.build()?))
        }
        MenuItemKind::Action => {
            let enabled = !item.disabled.unwrap_or(false);
            let menu_item = MudaMenuItem::with_id(
                app,
                item.id.as_str(),
                item.label.as_str(),
                enabled,
                None::<&str>,
            )?;
            Ok(builder.push(&menu_item))
        }
        MenuItemKind::Check => {
            let menu_item = CheckMenuItem::with_id(
                app,
                item.id.as_str(),
                item.label.as_str(),
                true,
                item.checked.unwrap_or(false),
                None::<&str>,
            )?;
            Ok(builder.push(&menu_item))
        }
    }
}

/// 当前菜单快照(menu-state 事件与 menu_snapshot 命令共用同一构建):
/// 收集各事实源状态 → menu::build_snapshot(纯函数)。
pub(crate) fn current_snapshot(app: &AppHandle) -> menu::MenuSnapshot {
    let t = locales::shell_texts(locales::detect_lang());
    let state = collect_menu_state(app);
    menu::build_snapshot(&t, &state)
}

/// 快照状态输入收集(事实源:theme.rs / autostart.rs / 升级槽位 /
/// upgrade.rs 流水线 / close.rs;快照是投影,不复制状态)。
fn collect_menu_state(app: &AppHandle) -> menu::MenuState {
    let (app_version, dsh_version) = update_slots();
    // 「升级中 disabled」:任一流水线在途即置灰——dsh 升级 Active 或应用升级
    // 下载/就绪(#38 只有 dsh;本票 #39 按 #31「消除静默失败」补齐应用侧,
    // 与 on_check_update 的 toast 守卫同源:UI 先于点击诚实呈现)
    let upgrade_running = any_upgrade_running(app);
    menu::MenuState::new(
        theme::current_choice(),
        autostart::current(),
        app_version,
        dsh_version,
        upgrade_running,
        close::current(),
    )
}

/// 任一升级流水线在途(dsh 升级 Active / 应用升级下载或就绪)。
/// 手动「检查更新」的 no-op 守卫与菜单 disabled 的同源事实(dialog.rs toast 化)。
fn any_upgrade_running(app: &AppHandle) -> bool {
    app.try_state::<upgrade::UpgradeManager>()
        .map(|m| m.inner().is_pipeline_running())
        .unwrap_or(false)
        || app
            .try_state::<update::UpdateManager>()
            .map(|m| m.inner().is_active())
            .unwrap_or(false)
}

/// 按当前状态重建快照并应用到两个投影(托盘 muda + menu-state 事件),
/// 同步徽标与 tooltip。任何状态变化(主题 / 自启 / 升级槽位 / 关闭行为 /
/// 升级流水线启停)后调用:Windows 勾选菜单项不会自动互斥,重建让三个勾选
/// 回到内存事实源(theme.rs / close.rs),避免视觉漂移。
pub(crate) fn refresh_menu(app: &AppHandle) {
    let snapshot = current_snapshot(app);
    let menu = build_menu(app, &snapshot);
    if let Some(tray) = TRAY.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tray.set_menu(menu.ok());
    }
    // 徽标 + tooltip:任一槽位非空即徽标变体;tooltip 优先 dsh(主产品)
    let (app_version, dsh_version) = update_slots();
    let t = locales::shell_texts(locales::detect_lang());
    let badge = app_version.is_some() || dsh_version.is_some();
    let tooltip = match dsh_version.as_deref() {
        Some(v) => t.tray_tooltip_dsh_available(v),
        None => app_version
            .as_deref()
            .map(|v| t.tray_tooltip_available(v))
            .unwrap_or_else(|| "DeepSeek Desktop".to_string()),
    };
    if let Some(tray) = TRAY.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
        let _ = tray.set_icon(Some(if badge { badge_icon() } else { normal_icon() }));
        // macOS:菜单栏图标按 template 渲染(黑+透明,深浅菜单栏自动适配);
        // set_icon 后须同步 template 状态(两方法均跨平台可调用,内部按平台生效)
        let _ = tray.set_icon_as_template(cfg!(target_os = "macos"));
        let _ = tray.set_tooltip(Some(tooltip));
    }
    // 壳菜单条镜像:同一快照随 menu-state 下发(前端先注册监听再拉快照,
    // 后到者覆盖,与 theme/update 同款竞态语义,#33)
    let _ = app.emit_to("main", "menu-state", &snapshot);
}

fn update_slots() -> (Option<String>, Option<String>) {
    let app_version = APP_UPDATE_VERSION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let dsh_version = DSH_UPDATE_VERSION
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    (app_version, dsh_version)
}

/// 设置应用自身升级通知槽位(Some = 发现新版,None = 清除;由 update.rs 调用)。
/// 徽标/菜单/tooltip 在 refresh_menu 统一呈现(#3 §1 通知形态)。
pub fn set_app_update(app: &AppHandle, version: Option<&str>) {
    if let Ok(mut g) = APP_UPDATE_VERSION.lock() {
        *g = version.map(String::from);
    }
    log::info!("[tray] 应用升级槽位 → {version:?}");
    refresh_menu(app);
}

/// 设置 dsh 升级通知槽位(Some = 发现新版,None = 清除;由 upgrade.rs 调用)。
/// 徽标/菜单/tooltip 在 refresh_menu 统一呈现(#3 §1 通知形态)。
pub fn set_dsh_update(app: &AppHandle, version: Option<&str>) {
    if let Ok(mut g) = DSH_UPDATE_VERSION.lock() {
        *g = version.map(String::from);
    }
    log::info!("[tray] dsh 升级槽位 → {version:?}");
    refresh_menu(app);
}

fn normal_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/32x32.png")
}

/// 徽标变体:右上角圆点(形状表达,不依赖颜色,深浅任务栏均可见——#3 §1)。
fn badge_icon() -> tauri::image::Image<'static> {
    tauri::include_image!("icons/32x32-badge.png")
}

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, &current_snapshot(app))?;
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(normal_icon())
        // macOS 菜单栏惯例:图标按 template 渲染(黑白自适应深浅菜单栏)
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&menu)
        // 左键不弹菜单,留给"切换显隐";右键仍弹菜单
        .show_menu_on_left_click(false)
        // 动作分发收敛到 dispatch_menu_action(与前端 menu_action 命令同表,#33)
        .on_menu_event(|app, event| dispatch_menu_action(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;
    if let Ok(mut g) = TRAY.lock() {
        *g = Some(tray);
    }
    Ok(())
}

/// 主题菜单点击分发:状态变更收敛到 theme::choose(单一事实源,负责持久化与
/// `theme-changed` 下发);本处按 #9 契约推 `tray-theme` 选择串事件,并重建
/// 菜单让三个勾选回到内存事实源(Windows 勾选项不自动互斥,见 refresh_menu)。
fn on_theme_chosen(app: &AppHandle, choice: ThemeChoice) {
    theme::choose(app, choice);
    refresh_menu(app);
    log::info!("[tray] 主题切换: {choice:?} → 推 tray-theme 事件");
    let _ = app.emit_to("main", "tray-theme", choice.event_payload());
}

/// 自启菜单点击分发:切换收敛到 autostart::set(唯一写入口,插件失败时内存
/// 保持 OS 实际状态);重建菜单让勾选回到内存事实源(Windows 勾选项不自动
/// 切换,与主题同款处理,见 refresh_menu)。
fn on_autostart_toggled(app: &AppHandle) {
    let next = !autostart::current();
    log::info!("[tray] 切换开机自启 → {next}");
    let _ = autostart::set(app, next);
    refresh_menu(app);
}

/// 统一动作分发(#33 拍板):托盘 on_menu_event 与前端 `menu_action` 命令
/// 共用同一张 id → 动作表,两处入口、一张动作表。theme_choice_from_id 并入。
pub(crate) fn dispatch_menu_action(app: &AppHandle, id: &str) {
    match id {
        "toggle" => toggle_window(app),
        id if id.starts_with("theme-") => {
            if let Some(choice) = theme_choice_from_id(id) {
                // 主题勾选状态由 refresh_menu 在重建时按内存事实源恢复,
                // 此处只负责状态变更与事件下发
                on_theme_chosen(app, choice);
            }
        }
        "close-ask" | "close-minimize" | "close-quit" => {
            if let Some(behavior) = close_behavior_from_id(id) {
                // 关闭行为:状态变更收敛到 close::set(单一事实源,负责持久化);
                // 菜单勾选随 refresh_menu 重建(close.rs 模块文档,#38)
                log::info!("[tray] 关闭行为 → {behavior:?}");
                close::set(app, behavior);
                refresh_menu(app);
            }
        }
        autostart::MENU_ID => on_autostart_toggled(app),
        "upgrade-dsh" | "upgrade-available" => {
            // 被动通知入口(#3 §1,两层共用):显示窗口 + 推卡片请求事件,
            // 前端按状态渲染对应卡片浮层(壳页常驻,无整窗导航,#36;
            // available 态需此显式请求才弹卡片,自动检测只亮徽标)
            let card = if id == "upgrade-dsh" {
                "upgrade-card-request"
            } else {
                "update-card-request"
            };
            log::info!("[tray] 菜单[升级] → 显示窗口 + 推卡片请求 {card}");
            show_main_window(app);
            let _ = app.emit_to("main", card, ());
        }
        "check-update" => {
            // #3 事件契约变更 + #17 组合编排:检查逻辑全在 Rust 侧,
            // 两层升级共用托盘手动入口,直接回答(见 on_check_update;
            // 流水线在途时 on_check_update 内部 no-op,快照同步 disabled,#38)
            log::info!("[tray] 检查更新(手动)");
            on_check_update(app);
        }
        "quit" => {
            dsh::set_quitting();
            if let Some(m) = app.try_state::<dsh::DshManager>() {
                dsh::kill_child(m.inner());
            }
            app.exit(0);
        }
        _ => {}
    }
}

/// 菜单快照命令:壳菜单条挂载时拉当前快照(先注册监听再 invoke,与
/// theme_state 同款「后到者覆盖,来自同一状态」竞态语义,#33)。
#[tauri::command]
pub async fn menu_snapshot(app: tauri::AppHandle) -> Result<menu::MenuSnapshot, String> {
    Ok(current_snapshot(&app))
}

/// 菜单动作命令:前端点击菜单项 → invoke 回流,Rust 侧与托盘 on_menu_event
/// 走同一分发函数(两处入口、一张动作表,#33)。
#[tauri::command]
pub async fn menu_action(app: tauri::AppHandle, id: String) -> Result<(), String> {
    dispatch_menu_action(&app, &id);
    Ok(())
}

/// 托盘「检查更新」手动入口的组合编排(#3 §1「直接回答」+ #17 两层共用触发;
/// #31 拍板 / #39 施工:原生对话框全部改为 shell-dialog Web UI 弹窗/toast):
///
/// 1. 任一升级流水线在途(dsh 或应用)→ toast「升级正在进行中」
///    (#31 行为修正:原静默 no-op 改可见反馈;菜单快照同步 disabled);
/// 2. dsh 层先答(`upgrade::manual_check`):dsh 新版 → AlertDialog [升级][稍后]
///    (boot 未就绪时只亮徽标,继续应用层);检查失败 → toast「检查更新失败」;
///    已用弹窗回答 → 结束,不再弹应用层弹窗(避免叠加);
/// 3. 应用层兜底(`update::check_now` 结果回调):应用新版 → AlertDialog
///    (附 release notes 摘要);无新版 → toast 合并「已是最新」(附 dsh 版本,
///    一次回答两层);应用检查失败 → toast「检查更新失败」。
fn on_check_update(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        // 1. 流水线在途守卫(#3 边界 + #31 行为修正:可见反馈取代静默 no-op)
        if any_upgrade_running(&app) {
            log::info!("[tray] 升级流水线在途,手动检查 → toast 可见反馈(#31)");
            dialog::toast_upgrade_running(&app);
            return;
        }
        // 2. dsh 层(同步检查,3-5s 超时;已用弹窗回答即结束)
        let boot_ready = app
            .try_state::<dsh::DshManager>()
            .map(|m| m.inner().phase() == dsh::Phase::Ready)
            .unwrap_or(false);
        let dsh_version = match upgrade::manual_check(&app) {
            upgrade::CheckResult::Found { version, current_version } if boot_ready => {
                dialog::show_upgrade_found(&app, &version, &current_version);
                return;
            }
            upgrade::CheckResult::Found { .. } => {
                // boot 未就绪:发现新版只亮徽标(确认也会被流水线守卫拒绝),
                // 继续应用层检查(原 manual_check 语义)
                None
            }
            upgrade::CheckResult::Failed => {
                dialog::toast_check_failed(&app);
                return;
            }
            upgrade::CheckResult::None { current_version } => current_version,
        };
        // 3. 应用层(异步,结果经回调做弹窗/toast 决策)
        if let Some(m) = app.try_state::<update::UpdateManager>() {
            let app2 = app.clone();
            m.inner().check_now(true, Some(Box::new(move |r| match r {
                update::ManualCheckResult::Found {
                    version,
                    current_version,
                    notes,
                } => {
                    dialog::show_update_found(&app2, &version, &current_version, notes.as_deref());
                }
                update::ManualCheckResult::None => {
                    dialog::toast_up_to_date(&app2, dsh_version.as_deref());
                }
                update::ManualCheckResult::Failed => {
                    dialog::toast_check_failed(&app2);
                }
            })));
        }
    });
}

/// shell-dialog 弹窗回答的分发(#31 拍板:与托盘 on_menu_event / menu_action
/// 同一张动作表——本函数与 dispatch_menu_action 同文件同风格,dialog.rs
/// 的 `shell_dialog_respond` 命令收敛到此)。kind/choice 未知与「稍后/取消」
/// 一律无操作(保持升级槽位/状态现状,等待下一次触发)。
pub(crate) fn dispatch_dialog_response(app: &AppHandle, kind: &str, choice: &str, remember: bool) {
    match (kind, choice) {
        // 发现应用新版 [升级] → 显示窗口 + 自动开始下载(#3:确认即授权,
        // 不二次确认;下载进度浮层随 update-state Downloading 自动出现)
        ("update-found", "upgrade") => {
            log::info!("[tray] 弹窗[升级](应用) → 显示窗口 + 自动开始下载");
            if let (Some(u), Some(d)) = (
                app.try_state::<update::UpdateManager>(),
                app.try_state::<dsh::DshManager>(),
            ) {
                u.inner().apply_now(d.inner());
            }
        }
        // 发现 dsh 新版 [升级] → 显示窗口 + 自动开始流水线(#3 §1:确认即授权,
        // 不二次确认;流水线进入 Active 后升级覆盖层自动出现,#32)
        ("upgrade-found", "upgrade") => {
            log::info!("[tray] 弹窗[升级](dsh) → 显示窗口 + 自动开始流水线");
            if let (Some(u), Some(d)) = (
                app.try_state::<upgrade::UpgradeManager>(),
                app.try_state::<dsh::DshManager>(),
            ) {
                u.inner().confirm_start(d.inner());
            }
        }
        // 关闭三选(首次 / 每次询问,#31 场景 5):执行去向;勾选「记住我的选择」
        // → close::set 持久化(下次直接执行不再弹,close.rs 单一事实源)
        ("close-ask", "minimize") => {
            dsh::reset_dialog_flag();
            close::execute(app, CloseBehavior::Minimize);
            if remember {
                log::info!("[tray] 关闭选择[最小化到托盘] + 记住 → 持久化");
                close::set(app, CloseBehavior::Minimize);
            }
        }
        ("close-ask", "quit") => {
            dsh::reset_dialog_flag();
            close::execute(app, CloseBehavior::Quit);
            if remember {
                log::info!("[tray] 关闭选择[退出] + 记住 → 持久化");
                close::set(app, CloseBehavior::Quit);
            }
        }
        ("close-ask", "cancel") => {
            // 取消:保持现状(窗口未关闭),仅复位防双触发守卫
            dsh::reset_dialog_flag();
        }
        _ => {}
    }
}

/// 显示并聚焦主窗口(取消最小化)。托盘动态升级菜单项与手动检查对话框
/// [升级] 共用——壳页常驻后不再整窗导航,「看升级卡片」= 显示窗口 +
/// 推卡片请求事件(或流水线状态自动弹卡)。
pub(crate) fn show_main_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

/// 显示/隐藏窗口。行为:窗口可见且已聚焦时隐藏,否则显示并聚焦。
/// 左键单击与菜单项共用同一语义。
fn toggle_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let visible = win.is_visible().unwrap_or(false);
    if visible && win.is_focused().unwrap_or(false) {
        let _ = win.hide();
    } else {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_choice_from_id_maps_all_menu_ids() {
        assert_eq!(theme_choice_from_id("theme-light"), Some(ThemeChoice::Light));
        assert_eq!(theme_choice_from_id("theme-dark"), Some(ThemeChoice::Dark));
        assert_eq!(theme_choice_from_id("theme-system"), Some(ThemeChoice::System));
        assert_eq!(theme_choice_from_id("theme-foo"), None);
        assert_eq!(theme_choice_from_id("toggle"), None);
    }

    #[test]
    fn theme_menu_id_and_payload_roundtrip() {
        // 不变量:菜单 id ↔ 选择 ↔ 前端事件 payload 一一对应,互不漂移
        // (映射的单一事实源在 theme.rs,这里守住托盘侧的不变量)
        for choice in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
            assert_eq!(theme_choice_from_id(choice.menu_id()), Some(choice));
            assert!(!choice.event_payload().is_empty());
        }
        // payload 是前端契约:固定小写英文串
        assert_eq!(ThemeChoice::Light.event_payload(), "light");
        assert_eq!(ThemeChoice::Dark.event_payload(), "dark");
        assert_eq!(ThemeChoice::System.event_payload(), "system");
    }

    #[test]
    fn close_behavior_from_id_maps_all_menu_ids() {
        // 与 theme_choice_from_id 同款不变量:菜单 id ↔ 选择 一一对应,互不漂移
        // (映射的单一事实源在 close.rs,这里守住托盘侧的不变量)
        for behavior in [CloseBehavior::Ask, CloseBehavior::Minimize, CloseBehavior::Quit] {
            assert_eq!(close_behavior_from_id(behavior.menu_id()), Some(behavior));
        }
        assert_eq!(close_behavior_from_id("close-foo"), None);
        assert_eq!(close_behavior_from_id("toggle"), None);
    }
}
