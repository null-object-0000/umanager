//! 桌面会话类型检测（X11 / Wayland）。
//!
//! 据此在前端给出不同的能力提示：X11 下应用内全局热键可用；Wayland 下
//! 普通应用无法全局抢占按键，应引导用户走系统自定义快捷键（GNOME 会替我们调用
//! `umanager --toggle-clipboard-panel`）。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    Wayland,
    X11,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub kind: SessionKind,
    pub wayland_display: Option<String>,
    pub display: Option<String>,
    pub session_type: Option<String>,
    pub global_hotkey_supported: bool,
}

fn detect_kind() -> SessionKind {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        SessionKind::Wayland
    } else if std::env::var_os("DISPLAY").is_some() {
        SessionKind::X11
    } else {
        SessionKind::Unknown
    }
}

#[tauri::command]
pub fn get_session_info() -> SessionInfo {
    let kind = detect_kind();
    SessionInfo {
        kind,
        wayland_display: std::env::var("WAYLAND_DISPLAY").ok(),
        display: std::env::var("DISPLAY").ok(),
        session_type: std::env::var("XDG_SESSION_TYPE").ok(),
        global_hotkey_supported: kind == SessionKind::X11,
    }
}
