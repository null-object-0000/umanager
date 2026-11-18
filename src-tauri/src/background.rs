//! 主窗口恢复与关闭。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, Window, WindowEvent};

static QUITTING: AtomicBool = AtomicBool::new(false);

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

/// 真正退出：有安装/卸载在跑时先拦下来，避免中途把任务中心一起关掉。
fn quit_app(app: &AppHandle) {
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

/// 关闭主窗口时，由界面根据任务状态决定是否提示确认。
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
