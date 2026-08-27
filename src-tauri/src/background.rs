//! 后台常驻：托盘图标 + 全局热键 + 关闭窗口时收起而非退出。
//!
//! 全局热键默认唤出剪贴板快捷面板（`panel`），托盘菜单可显隐面板/主窗口/退出。
//! 热键字符串持久化到应用配置目录，可在运行期通过命令修改。
//!
//! 平台说明：
//! - Linux 托盘（libayatana-appindicator）只支持菜单，不触发裸点击事件。
//! - 全局热键在 X11 可用；Wayland 上合成器一般不允许，注册失败仅记日志。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Window, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutEvent, ShortcutState};

use crate::panel;

const DEFAULT_SHORTCUT: &str = "Super+V";

static QUITTING: AtomicBool = AtomicBool::new(false);
static DRAGGING: AtomicBool = AtomicBool::new(false);

/// 托盘图标必须持有到进程结束，否则会被立即回收。
struct BackgroundState {
    _tray: tauri::tray::TrayIcon,
}

/// 当前热键字符串（运行期可改并持久化）。
pub struct HotkeyState(Mutex<String>);

#[derive(serde::Serialize, serde::Deserialize)]
struct ClipboardSettings {
    #[serde(default = "default_hotkey")]
    hotkey: String,
}

fn default_hotkey() -> String {
    DEFAULT_SHORTCUT.to_string()
}

fn shortcut_handler(
    app: &AppHandle,
    _shortcut: &tauri_plugin_global_shortcut::Shortcut,
    event: ShortcutEvent,
) {
    if event.state() == ShortcutState::Pressed {
        panel::toggle(app);
    }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join("clipboard-settings.json"))
}

fn load_hotkey(app: &AppHandle) -> String {
    settings_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<ClipboardSettings>(&raw).ok())
        .map(|settings| settings.hotkey)
        .filter(|hotkey| !hotkey.trim().is_empty())
        .unwrap_or_else(default_hotkey)
}

fn persist_hotkey(app: &AppHandle, hotkey: &str) {
    let Some(path) = settings_path(app) else {
        return;
    };
    if let Some(directory) = path.parent() {
        let _ = std::fs::create_dir_all(directory);
    }
    if let Ok(json) = serde_json::to_string(&ClipboardSettings {
        hotkey: hotkey.to_string(),
    }) {
        let _ = std::fs::write(path, json);
    }
}

fn register_hotkey(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(hotkey, shortcut_handler)
        .map_err(|error| {
            eprintln!("注册全局热键 {hotkey} 失败（Wayland 上属预期）：{error}");
            error.to_string()
        })
}

/// 初始化托盘与全局热键。应在 `.setup()` 中调用一次。
pub fn initialize(app: &AppHandle) -> tauri::Result<()> {
    let panel_item = MenuItem::with_id(app, "panel", "打开剪贴板面板", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "显示 UManager", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&panel_item, &show, &quit])?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().expect("缺少应用图标"))
        .tooltip("UManager · 剪贴板历史运行中")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "panel" => panel::toggle(app),
            "show" => show_window(app),
            "quit" => {
                QUITTING.store(true, Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    let hotkey = load_hotkey(app);
    if let Err(error) = register_hotkey(app, hotkey.as_str()) {
        eprintln!("{error}");
    }
    app.manage(HotkeyState(Mutex::new(hotkey)));
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

/// 标记「正在从剪贴板历史拖拽文件出去」，供面板失焦收起逻辑避开拖拽期间。
pub fn mark_dragging(dragging: bool) {
    DRAGGING.store(dragging, Ordering::SeqCst);
}

fn is_dragging() -> bool {
    DRAGGING.load(Ordering::SeqCst)
}

/// 关闭窗口时收起而非退出；快捷面板失焦后自动收起（拖拽期间除外）。
pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            if !is_quitting() {
                api.prevent_close();
                let _ = window.hide();
            }
        }
        WindowEvent::Focused(false) => {
            if window.label() == panel::PANEL_LABEL && !is_dragging() {
                panel::hide(window.app_handle());
            }
        }
        _ => {}
    }
}

#[tauri::command]
pub fn get_clipboard_hotkey(state: tauri::State<'_, HotkeyState>) -> Result<String, String> {
    state
        .0
        .lock()
        .map(|hotkey| hotkey.clone())
        .map_err(|_| "热键状态不可用".to_string())
}

#[tauri::command]
pub fn set_clipboard_hotkey(
    app: AppHandle,
    state: tauri::State<'_, HotkeyState>,
    hotkey: String,
) -> Result<String, String> {
    let new_hotkey = hotkey.trim().to_string();
    if new_hotkey.is_empty() {
        return Err("热键不能为空".to_string());
    }
    let old_hotkey = state
        .0
        .lock()
        .map(|hotkey| hotkey.clone())
        .map_err(|_| "热键状态不可用".to_string())?;

    let _ = app.global_shortcut().unregister(old_hotkey.as_str());
    if let Err(error) = app
        .global_shortcut()
        .on_shortcut(new_hotkey.as_str(), shortcut_handler)
    {
        eprintln!("注册全局热键 {new_hotkey} 失败：{error}");
        let _ = app
            .global_shortcut()
            .on_shortcut(old_hotkey.as_str(), shortcut_handler);
        return Err(format!("无法注册热键 {new_hotkey}：{error}"));
    }

    if let Ok(mut current) = state.0.lock() {
        *current = new_hotkey.clone();
    }
    persist_hotkey(&app, &new_hotkey);
    Ok(new_hotkey)
}
