//! 剪贴板快捷面板。
//!
//! 一个无边框、置顶、不占任务栏的小窗口（在 `tauri.conf.json` 中声明，启动即隐藏），
//! 由全局热键或托盘菜单唤起，定位到主显示器的可用区域右上角（托盘区附近），
//! 失焦后自动收起。

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

pub const PANEL_LABEL: &str = "clipboard-panel";
/// 从命令行或 GNOME 自定义快捷键触发面板显隐的开关参数。
pub const TOGGLE_ARG: &str = "--toggle-clipboard-panel";
const EDGE_MARGIN: i32 = 12;
const TOP_MARGIN: i32 = 12;

/// 显示 / 隐藏快捷面板。
pub fn toggle(app: &AppHandle) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
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
    }
}

/// 隐藏快捷面板（无操作时静默忽略）。
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        let _ = window.hide();
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
        .or_else(|| main.as_ref().and_then(|target| target.primary_monitor().ok().flatten()))
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
