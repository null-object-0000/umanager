//! 后台常驻：托盘图标与关闭窗口时收起。

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Window, WindowEvent,
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

    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().expect("缺少应用图标"))
        .tooltip("UManager")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "quit" => {
                if crate::operation_plan::MUTATION_LOCK.try_lock().is_err() {
                    show_window(app);
                    let _ = notify_rust::Notification::new().summary("软件操作尚未完成").body("请等待安装、卸载或批量复核完成后再退出 UManager。").show();
                    return;
                }
                QUITTING.store(true, Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    app.manage(BackgroundState { _tray: tray });
    Ok(())
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// 关闭主窗口时收起到托盘，让后台任务继续运行。
pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if !is_quitting() {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}
