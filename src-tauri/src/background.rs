//! 后台常驻：托盘图标 + 全局热键 + 关闭窗口时收起而非退出。
//!
//! 托盘菜单里会放最近几条剪贴板文本（点击即复制）。在 Linux（appindicator）上，
//! 托盘菜单由桌面锚定在托盘图标旁，是 Wayland 下唯一能「贴着托盘」呈现的形态；
//! 独立快捷面板窗口在 Wayland 上无法被应用自行定位到托盘旁。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{
    menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Window, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutEvent, ShortcutState};

use crate::clipboard_history::{ClipboardEntry, ClipboardHistory, ClipboardKind};
use crate::panel;

const DEFAULT_SHORTCUT: &str = "Super+V";
/// 托盘菜单里展示的最近文本条数。
const TRAY_CLIPBOARD_ITEMS: usize = 6;

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

/// 托盘菜单条目文本：压成单行并截断。
fn tray_entry_label(entry: &ClipboardEntry) -> String {
    let one_line = entry
        .text
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = one_line.chars();
    let short: String = chars.by_ref().take(48).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

/// 构造托盘菜单（须在主线程调用）。
fn build_tray_menu(app: &AppHandle) -> Option<Menu<tauri::Wry>> {
    let snapshot = app.state::<ClipboardHistory>().snapshot();

    let mut clip_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    for entry in snapshot
        .iter()
        .filter(|entry| entry.kind == ClipboardKind::Text)
        .take(TRAY_CLIPBOARD_ITEMS)
    {
        if let Ok(item) = MenuItem::with_id(
            app,
            format!("clip-{}", entry.id),
            tray_entry_label(entry),
            true,
            None::<&str>,
        ) {
            clip_items.push(item);
        }
    }

    let open_panel = MenuItem::with_id(app, "panel", "打开剪贴板面板", true, None::<&str>).ok()?;
    let show_main = MenuItem::with_id(app, "show", "显示 UManager", true, None::<&str>).ok()?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>).ok()?;
    let separator_top = PredefinedMenuItem::separator(app).ok()?;
    let separator_bottom = PredefinedMenuItem::separator(app).ok()?;

    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = Vec::new();
    for item in &clip_items {
        items.push(item as &dyn IsMenuItem<tauri::Wry>);
    }
    if !items.is_empty() {
        items.push(&separator_top as &dyn IsMenuItem<tauri::Wry>);
    }
    items.push(&open_panel as &dyn IsMenuItem<tauri::Wry>);
    items.push(&show_main as &dyn IsMenuItem<tauri::Wry>);
    items.push(&separator_bottom as &dyn IsMenuItem<tauri::Wry>);
    items.push(&quit as &dyn IsMenuItem<tauri::Wry>);

    Menu::with_items(app, &items).ok()
}

/// 重建托盘菜单（须在主线程调用）。
pub fn refresh_tray_menu(app: &AppHandle) {
    let Some(menu) = build_tray_menu(app) else {
        return;
    };
    if let Some(tray) = app.tray_by_id("main-tray") {
        if let Err(error) = tray.set_menu(Some(menu)) {
            eprintln!("更新托盘菜单失败：{error}");
        }
    }
}

/// 任意线程安全：把托盘菜单重建调度到主线程。
pub fn schedule_tray_menu_refresh(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || refresh_tray_menu(&handle));
}

fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id.as_ref().to_string();
    match id.as_str() {
        "panel" => panel::toggle(app),
        "show" => show_window(app),
        "quit" => {
            QUITTING.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        _ => {
            if let Some(raw) = id.strip_prefix("clip-") {
                if let Ok(entry_id) = raw.parse::<u64>() {
                    if let Err(error) = app.state::<ClipboardHistory>().copy_text(entry_id) {
                        eprintln!("托盘复制剪贴板条目失败：{error}");
                    }
                }
            }
        }
    }
}

/// 初始化托盘与全局热键。应在 `.setup()` 中调用一次。
pub fn initialize(app: &AppHandle) -> tauri::Result<()> {
    let open_panel = MenuItem::with_id(app, "panel", "打开剪贴板面板", true, None::<&str>)?;
    let show_main = MenuItem::with_id(app, "show", "显示 UManager", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let initial_menu = Menu::with_items(app, &[&open_panel, &show_main, &quit])?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().expect("缺少应用图标"))
        .tooltip("UManager · 剪贴板历史运行中")
        .menu(&initial_menu)
        .on_menu_event(handle_menu_event)
        .build(app)?;

    let hotkey = load_hotkey(app);
    if let Err(error) = register_hotkey(app, hotkey.as_str()) {
        eprintln!("{error}");
    }
    app.manage(HotkeyState(Mutex::new(hotkey)));
    app.manage(BackgroundState { _tray: tray });

    // 用最近的剪贴板条目填充托盘菜单。
    refresh_tray_menu(app);
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
