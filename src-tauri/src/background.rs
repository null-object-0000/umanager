//! 后台常驻：托盘图标与「关闭窗口」的语义。
//!
//! 结论先写在前面：**不能相信托盘图标一定看得见**。GNOME 不装 AppIndicator
//! 扩展时，libayatana-appindicator 照样能加载、图标对象照样能创建，但用户永远
//! 看不到它；此时若把窗口 hide 进托盘，窗口就再也叫不回来，看起来正是
//! 「点关闭按钮没反应」。
//!
//! 因此关闭窗口一律**不隐藏**：
//!
//! * 有未完成的下载/批量任务 → 界面弹确认框，说明关掉会中断什么；
//! * 没有未完成任务 → 直接退出；
//! * 想留在后台时，走托盘菜单（装了状态栏扩展才有）。
//!
//! 托盘图标只是「有就能用」的加分项，不再是关闭按钮的前提，初始化失败也不会
//! 影响主窗口启动。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Window, WindowEvent,
};

static QUITTING: AtomicBool = AtomicBool::new(false);
/// 托盘图标必须持有到进程结束，否则会被立即回收。
struct BackgroundState {
    _tray: tauri::tray::TrayIcon,
}

/// 初始化托盘。应在 `.setup()` 中调用一次。
pub fn initialize(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示 UManager", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    // 托盘只是加分项：初始化失败（缺 appindicator 库、没有状态栏宿主体等）不能
    // 让整个应用起不来，否则用户连主窗口都看不到。失败时照常继续，关闭按钮走
    // 确认后退出这条路径。
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().expect("缺少应用图标"))
        .tooltip("UManager")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "quit" => quit_app(app),
            _ => {}
        })
        .build(app);

    match tray {
        Ok(tray) => {
            app.manage(BackgroundState { _tray: tray });
        }
        Err(error) => eprintln!("托盘图标初始化失败，将继续以无托盘方式运行：{error}"),
    }
    Ok(())
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// 真正退出：有安装/卸载在跑时先拦下来，避免中途把任务中心一起关掉。
pub fn quit_app(app: &AppHandle) {
    // 命令本身在 GTK 主线程里同步执行，持锁的异步操作无法在此完成，
    // 所以这里只做一次不会真正等待的尝试，避免正常操作被误判为占用。
    if tokio::sync::Mutex::try_lock(&crate::operation_plan::MUTATION_LOCK).is_err() {
        show_window(app);
        let _ = notify_rust::Notification::new()
            .summary("软件操作尚未完成")
            .body("请等待安装、卸载或批量复核完成后再退出 UManager。")
            .show();
        return;
    }
    QUITTING.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// 关闭主窗口：托盘不可信，所以不隐藏，而是先让界面确认再退出。
pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if !is_quitting() {
            api.prevent_close();
            // 界面弹出确认框；用户确认后走 confirm_close。界面在时间窗内没回应
            // （例如前端卡死）就自己退出，避免再次出现「点 × 没反应」。
            let _ = window.emit("close-requested", ());
            let app = window.app_handle().clone();
            let generation = CLOSE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(CLOSE_CONFIRM_TIMEOUT_SECONDS)).await;
                // 用户点了「继续使用」或又关了一次，这个计时器就不再作数。
                if CLOSE_GENERATION.load(Ordering::SeqCst) == generation && !is_quitting() {
                    quit_app(&app);
                }
            });
        }
    }
}

/// 等界面确认关闭的最长时间。
const CLOSE_CONFIRM_TIMEOUT_SECONDS: u64 = 30;
/// 每次把关闭转给界面时递增，用来作废上一次的兜底计时器。
static CLOSE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 用户在确认框里选择「继续使用」：作废待执行的兜底退出。
pub fn cancel_close() {
    CLOSE_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// 用户在「退出 UManager」确认框里点确认后调用；有未完成的安装/卸载会被拒绝。
#[tauri::command]
pub fn confirm_close(app: AppHandle) -> Result<(), String> {
    if crate::operation_plan::MUTATION_LOCK.try_lock().is_err() {
        return Err("软件操作尚未完成，请等安装或卸载结束后再退出。".to_owned());
    }
    QUITTING.store(true, Ordering::SeqCst);
    app.exit(0);
    Ok(())
}

/// 用户在「退出 UManager」确认框里点了「继续使用」。
#[tauri::command]
pub fn cancel_close_confirmation() {
    cancel_close();
}
