//! 剪贴板快捷面板。
//!
//! 一个无边框、置顶、不占任务栏的小窗口（在 `tauri.conf.json` 中声明，按需创建），
//! 由全局热键或托盘菜单唤起，定位到主显示器的可用区域右上角（托盘区附近），
//! 失焦后自动收起。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow, WebviewWindowBuilder};

use crate::clipboard_history;

pub const PANEL_LABEL: &str = "clipboard-panel";
/// 从命令行或 GNOME 自定义快捷键触发面板显隐的开关参数。
pub const TOGGLE_ARG: &str = "--toggle-clipboard-panel";
const EDGE_MARGIN: i32 = 12;
const TOP_MARGIN: i32 = 12;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
static ACTIVITY: AtomicU64 = AtomicU64::new(0);

/// 显示 / 隐藏快捷面板。
pub fn toggle(app: &AppHandle) {
    // 串行创建和销毁 GTK 窗口，也避免同步 command 线程上构建 WebView 的死锁。
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || toggle_on_main_thread(&handle));
}

fn toggle_on_main_thread(app: &AppHandle) {
    ACTIVITY.fetch_add(1, Ordering::SeqCst);
    let window = match app.get_webview_window(PANEL_LABEL) {
        Some(window) => window,
        None => {
            let Some(config) = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == PANEL_LABEL)
            else {
                return;
            };
            match WebviewWindowBuilder::from_config(app, config).and_then(|builder| builder.build())
            {
                Ok(window) => window,
                Err(error) => {
                    eprintln!("创建剪贴板面板失败：{error}");
                    return;
                }
            }
        }
    };
    if window.is_visible().unwrap_or(false) {
        hide(app);
    } else {
        let position = || {
            if let Err(error) = position_near_tray(app, &window) {
                eprintln!("定位剪贴板面板失败：{error}");
            }
        };
        position();
        let _ = window.show();
        // X11 下 show 前后都设一次更稳，避免个别 WM 只在映射时读取位置提示。
        position();
        let _ = window.set_focus();
        // 面板隐藏期间前端监听器可能收不到实时变更，显示后补发一次最新快照。
        clipboard_history::refresh(app);
    }
}

/// 隐藏快捷面板（无操作时静默忽略）。
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let _ = window.hide();
        let epoch = ACTIVITY.fetch_add(1, Ordering::SeqCst) + 1;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(IDLE_TIMEOUT).await;
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if ACTIVITY.load(Ordering::SeqCst) != epoch {
                    return;
                }
                if let Some(window) = handle.get_webview_window(PANEL_LABEL) {
                    if window.is_visible().is_ok_and(|visible| !visible) {
                        let _ = window.destroy();
                    }
                }
            });
        });
    }
}

/// 把面板定位到主窗口所在显示器的「可用区域右上角」（GNOME 顶栏/托盘就在那里）。
/// 注意：Wayland 下系统不允许普通应用给窗口定位，`set_position` 会被合成器忽略——
/// 这是平台限制，X11 下有效。
fn position_near_tray(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let main = app.get_webview_window("main");
    let monitor = main
        .as_ref()
        .and_then(|target| target.current_monitor().ok().flatten())
        .or_else(|| {
            main.as_ref()
                .and_then(|target| target.primary_monitor().ok().flatten())
        })
        .or_else(|| window.primary_monitor().ok().flatten())
        .or_else(|| window.monitor_from_point(0.0, 0.0).ok().flatten())
        .ok_or_else(|| "找不到显示器".to_string())?;
    let outer = window.outer_size().map_err(|error| error.to_string())?;
    let work = monitor.work_area();
    let x = (work.position.x + work.size.width as i32 - outer.width as i32 - EDGE_MARGIN)
        .max(work.position.x + EDGE_MARGIN);
    let y = work.position.y + TOP_MARGIN;
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn hide_clipboard_panel(app: AppHandle) {
    hide(&app);
}
