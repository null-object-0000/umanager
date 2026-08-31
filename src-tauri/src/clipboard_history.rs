//! 剪贴板历史（文本 + 图片）。
//!
//! Phase 1/2：应用运行期间在后台轮询系统剪贴板，把新复制到的文本和图片按「最新在前」
//! 存入内存并持久化到应用数据目录；前端页面负责浏览、搜索、置顶、回拷与删除。
//!
//! 设计要点：
//! - 文本与图片统一存进同一条 `entries` 列表；文本内联在 JSON，图片的原始 PNG 单文件
//!   存放（`clipboard-images/{id}.png`），JSON 只存元数据与一张小缩略图（base64 data URL）。
//! - 轮询（默认 800ms）优先尝试读图片（`arboard::get_image` 返回 RGBA8），读不到再读文本；
//!   读取失败（无聚焦、Wayland 无数据控制、剪贴板为空等）时静默跳过。
//! - 上限：最多 500 条、文本总字符 200 万、单条文本 20 万字符、单张图片编码后 ≤20MB、
//!   图片总字节 ≤300MB；淘汰时优先丢未置顶的最旧条目，并同步删除图片文件。
//! - 去重：文本按内容比较、图片按 PNG 的 SHA-256 比较；重复时移到最前并刷新时间。
//! - 持久化 JSON 向后兼容：旧版本只有文本字段，`kind` 缺省按「文本」解析。

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// 最多保留的条目数（文本与图片合计）。
const MAX_ENTRIES: usize = 500;
/// 全部文本条目的字符数上限。
const MAX_TOTAL_CHARS: usize = 2_000_000;
/// 单条文本的字符数上限；超长内容截断并加省略号标记。
const MAX_ENTRY_CHARS: usize = 200_000;
/// 单张图片编码为 PNG 后的字节数上限。
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// 全部图片条目的字节数上限。
const MAX_TOTAL_IMAGE_BYTES: u64 = 300 * 1024 * 1024;
/// 列表缩略图的最长边（像素）。
const THUMBNAIL_MAX: u32 = 240;
/// 后台轮询剪贴板的间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardKind {
    Text,
    Image,
}

impl Default for ClipboardKind {
    fn default() -> Self {
        ClipboardKind::Text
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardEntry {
    pub id: u64,
    #[serde(default)]
    pub kind: ClipboardKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_count: Option<usize>,
    pub pinned: bool,
    pub captured_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_byte_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_preview: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedHistory {
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    entries: Vec<ClipboardEntry>,
}

struct Store {
    next_id: u64,
    entries: Vec<ClipboardEntry>,
    data_path: Option<PathBuf>,
    image_dir: Option<PathBuf>,
}

/// Tauri 管理的剪贴板历史状态（`Send + Sync`）。
pub struct ClipboardHistory {
    inner: Mutex<Store>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let mut output = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut output), image::ImageFormat::Png)
        .ok()?;
    Some(output)
}

fn thumbnail_data_url(rgba: &[u8], width: u32, height: u32) -> Option<String> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let thumbnail = image::DynamicImage::ImageRgba8(image).thumbnail(THUMBNAIL_MAX, THUMBNAIL_MAX);
    let mut output = Vec::new();
    thumbnail
        .write_to(&mut Cursor::new(&mut output), image::ImageFormat::Png)
        .ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(output)
    ))
}

/// 生成拖拽时跟随鼠标的小预览图（最长边 160px），避免原图整张悬在光标下。
fn drag_preview_png(path: &Path) -> Vec<u8> {
    const PREVIEW_MAX: u32 = 160;
    std::fs::read(path)
        .ok()
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .and_then(|image| {
            let thumbnail = image.thumbnail(PREVIEW_MAX, PREVIEW_MAX);
            let mut output = Vec::new();
            thumbnail
                .write_to(&mut Cursor::new(&mut output), image::ImageFormat::Png)
                .ok()?;
            Some(output)
        })
        .unwrap_or_default()
}

fn load_image_data(path: &Path) -> Result<arboard::ImageData<'static>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("无法读取图片：{error}"))?;
    let image = image::load_from_memory(&bytes).map_err(|error| format!("无法解码图片：{error}"))?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: Cow::Owned(rgba.into_raw()),
    })
}

impl Store {
    fn snapshot(&self) -> Vec<ClipboardEntry> {
        self.entries.clone()
    }

    fn get(&self, id: u64) -> Result<ClipboardEntry, String> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| "未找到该条剪贴板记录".to_string())
    }

    fn image_path(&self, id: u64) -> Option<PathBuf> {
        self.image_dir
            .as_ref()
            .map(|directory| directory.join(format!("{id}.png")))
    }

    /// 捕获一次文本。返回新插入/被移到最前的条目；空文本返回 `None`。
    fn capture_text(&mut self, raw: &str) -> Option<ClipboardEntry> {
        let mut text = raw.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let mut char_count = text.chars().count();
        if char_count > MAX_ENTRY_CHARS {
            text = format!("{}…", text.chars().take(MAX_ENTRY_CHARS).collect::<String>());
            char_count = text.chars().count();
        }

        if let Some(position) = self
            .entries
            .iter()
            .position(|entry| entry.kind == ClipboardKind::Text && entry.text.as_deref() == Some(text.as_str()))
        {
            let mut entry = self.entries.remove(position);
            entry.captured_at_ms = now_ms();
            self.entries.insert(0, entry.clone());
            self.persist();
            return Some(entry);
        }

        let entry = ClipboardEntry {
            id: self.next_id,
            kind: ClipboardKind::Text,
            text: Some(text),
            char_count: Some(char_count),
            pinned: false,
            captured_at_ms: now_ms(),
            content_hash: None,
            image_width: None,
            image_height: None,
            image_byte_count: None,
            image_preview: None,
        };
        self.next_id += 1;
        self.entries.insert(0, entry.clone());
        self.enforce_caps();
        self.persist();
        Some(entry)
    }

    /// 捕获一次图片（RGBA8）。返回新插入/被移到最前的条目。
    fn capture_image(&mut self, rgba: &[u8], width: u32, height: u32) -> Option<ClipboardEntry> {
        if width == 0 || height == 0 {
            return None;
        }
        let png = encode_png(rgba, width, height)?;
        let byte_count = png.len() as u64;
        if byte_count > MAX_IMAGE_BYTES {
            return None;
        }
        let content_hash = sha256_hex(&png);

        if let Some(position) = self
            .entries
            .iter()
            .position(|entry| entry.kind == ClipboardKind::Image && entry.content_hash.as_deref() == Some(content_hash.as_str()))
        {
            let mut entry = self.entries.remove(position);
            entry.captured_at_ms = now_ms();
            self.entries.insert(0, entry.clone());
            self.persist();
            return Some(entry);
        }

        let id = self.next_id;
        self.next_id += 1;
        let image_dir = self.image_dir.clone()?;
        if std::fs::create_dir_all(&image_dir).is_err() {
            return None;
        }
        if std::fs::write(image_dir.join(format!("{id}.png")), &png).is_err() {
            return None;
        }

        let entry = ClipboardEntry {
            id,
            kind: ClipboardKind::Image,
            text: None,
            char_count: None,
            pinned: false,
            captured_at_ms: now_ms(),
            content_hash: Some(content_hash),
            image_width: Some(width),
            image_height: Some(height),
            image_byte_count: Some(byte_count),
            image_preview: thumbnail_data_url(rgba, width, height),
        };
        self.entries.insert(0, entry.clone());
        self.enforce_caps();
        self.persist();
        Some(entry)
    }

    fn set_pinned(&mut self, id: u64, pinned: bool) -> Result<(), String> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "未找到该条剪贴板记录".to_string())?;
        entry.pinned = pinned;
        self.persist();
        Ok(())
    }

    fn delete(&mut self, id: u64) -> Result<(), String> {
        let Some(position) = self.entries.iter().position(|entry| entry.id == id) else {
            return Err("未找到该条剪贴板记录".to_string());
        };
        let entry = self.entries.remove(position);
        self.remove_entry_file(&entry);
        self.persist();
        Ok(())
    }

    fn clear(&mut self) {
        for entry in &self.entries {
            self.remove_entry_file(entry);
        }
        self.entries.clear();
        self.persist();
    }

    fn remove_entry_file(&self, entry: &ClipboardEntry) {
        if entry.kind == ClipboardKind::Image {
            if let Some(path) = self.image_path(entry.id) {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn remove_at(&mut self, index: usize) {
        let entry = self.entries.remove(index);
        self.remove_entry_file(&entry);
    }

    /// 从尾部淘汰：优先淘汰未置顶的最旧条目；只剩置顶条目时才淘汰置顶。
    fn enforce_caps(&mut self) {
        while self.entries.len() > MAX_ENTRIES {
            let removable = self.entries.iter().rposition(|entry| !entry.pinned);
            let index = removable.unwrap_or(self.entries.len() - 1);
            self.remove_at(index);
        }

        let mut text_total: usize = self.entries.iter().filter_map(|entry| entry.char_count).sum();
        while text_total > MAX_TOTAL_CHARS {
            let Some(index) = self
                .entries
                .iter()
                .rposition(|entry| !entry.pinned && entry.kind == ClipboardKind::Text)
            else {
                break;
            };
            text_total -= self.entries[index].char_count.unwrap_or(0);
            self.remove_at(index);
        }

        let mut image_total: u64 = self
            .entries
            .iter()
            .filter_map(|entry| entry.image_byte_count)
            .sum();
        while image_total > MAX_TOTAL_IMAGE_BYTES {
            let Some(index) = self
                .entries
                .iter()
                .rposition(|entry| !entry.pinned && entry.kind == ClipboardKind::Image)
            else {
                break;
            };
            image_total -= self.entries[index].image_byte_count.unwrap_or(0);
            self.remove_at(index);
        }
    }

    fn persist(&self) {
        let Some(path) = self.data_path.as_ref() else {
            return;
        };
        let payload = PersistedHistory {
            next_id: self.next_id,
            entries: self.entries.clone(),
        };
        let Ok(json) = serde_json::to_string(&payload) else {
            return;
        };
        if let Some(directory) = path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }
        let temporary = path.with_extension("json.tmp");
        if std::fs::write(&temporary, json).is_ok() {
            let _ = std::fs::rename(&temporary, path);
        }
    }
}

impl ClipboardHistory {
    /// 从 `data_dir`（通常是 `app_data_dir()`）载入已有历史。
    pub fn load(data_dir: Option<PathBuf>) -> Self {
        let data_path = data_dir.as_ref().map(|directory| directory.join("clipboard-history.json"));
        let image_dir = data_dir.as_ref().map(|directory| directory.join("clipboard-images"));
        let mut store = Store {
            next_id: 1,
            entries: Vec::new(),
            data_path,
            image_dir,
        };
        if let Some(path) = store.data_path.as_ref() {
            if let Ok(raw) = std::fs::read_to_string(path) {
                if let Ok(persisted) = serde_json::from_str::<PersistedHistory>(&raw) {
                    let max_entry_id = persisted
                        .entries
                        .iter()
                        .map(|entry| entry.id)
                        .max()
                        .unwrap_or(0);
                    store.next_id = persisted.next_id.max(max_entry_id + 1).max(1);
                    store.entries = persisted.entries;
                }
            }
        }
        Self {
            inner: Mutex::new(store),
        }
    }

    pub fn snapshot(&self) -> Vec<ClipboardEntry> {
        self.inner
            .lock()
            .map(|store| store.snapshot())
            .unwrap_or_default()
    }

    pub fn get(&self, id: u64) -> Result<ClipboardEntry, String> {
        self.inner
            .lock()
            .map_err(|_| "剪贴板历史状态不可用".to_string())?
            .get(id)
    }

    pub fn capture_text(&self, text: &str) -> Option<ClipboardEntry> {
        self.inner
            .lock()
            .ok()
            .and_then(|mut store| store.capture_text(text))
    }

    pub fn capture_image(&self, rgba: &[u8], width: u32, height: u32) -> Option<ClipboardEntry> {
        self.inner
            .lock()
            .ok()
            .and_then(|mut store| store.capture_image(rgba, width, height))
    }

    pub fn image_path(&self, id: u64) -> Result<PathBuf, String> {
        self.inner
            .lock()
            .map_err(|_| "剪贴板历史状态不可用".to_string())?
            .image_path(id)
            .ok_or_else(|| "图片目录不可用".to_string())
    }

    pub fn set_pinned(&self, id: u64, pinned: bool) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|_| "剪贴板历史状态不可用".to_string())?
            .set_pinned(id, pinned)
    }

    pub fn delete(&self, id: u64) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|_| "剪贴板历史状态不可用".to_string())?
            .delete(id)
    }

    pub fn clear(&self) {
        if let Ok(mut store) = self.inner.lock() {
            store.clear();
        }
    }
}

/// 初始化状态并启动后台轮询线程。应在 `.setup()` 中调用一次。
pub fn initialize(app: &AppHandle) {
    let data_dir = app.path().app_data_dir().ok();
    let history = ClipboardHistory::load(data_dir);
    app.manage(history);
    start_monitor(app.clone());
}

fn emit_snapshot(app: &AppHandle, history: &ClipboardHistory) {
    let _ = app.emit("clipboard-history-changed", history.snapshot());
}

/// 面板每次显示时调用：把当前剪贴板快照重新推给所有窗口。
///
/// 快捷面板是一个常驻隐藏的 webview，隐藏期间前端监听器可能收不到实时变更；
/// 在 `panel::toggle` 显示面板后补发一次快照，保证面板一弹出就是最新数据，
/// 而不必等用户重启应用。
pub fn refresh(app: &AppHandle) {
    if let Some(history) = app.try_state::<ClipboardHistory>() {
        emit_snapshot(app, &history);
    }
}

fn start_monitor(app: AppHandle) {
    let _ = std::thread::Builder::new()
        .name("umanager-clipboard-monitor".to_string())
        .spawn(move || {
            let Ok(mut clipboard) = arboard::Clipboard::new() else {
                return;
            };
            let mut last_text: Option<String> = None;
            let mut last_image_hash: Option<String> = None;
            loop {
                std::thread::sleep(POLL_INTERVAL);

                if let Ok(image) = clipboard.get_image() {
                    let rgba = image.bytes;
                    let hash = sha256_hex(&rgba);
                    if last_image_hash.as_deref() == Some(hash.as_str()) {
                        continue;
                    }
                    last_image_hash = Some(hash);
                    last_text = None;
                    let history = app.state::<ClipboardHistory>();
                    if history
                        .capture_image(&rgba, image.width as u32, image.height as u32)
                        .is_some()
                    {
                        emit_snapshot(&app, &history);
                    }
                    continue;
                }

                if let Ok(text) = clipboard.get_text() {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if last_text.as_deref() == Some(trimmed) {
                        last_image_hash = None;
                        continue;
                    }
                    last_text = Some(trimmed.to_string());
                    last_image_hash = None;
                    let history = app.state::<ClipboardHistory>();
                    if history.capture_text(trimmed).is_some() {
                        emit_snapshot(&app, &history);
                    }
                }
            }
        });
}

#[tauri::command]
pub fn list_clipboard_history(state: tauri::State<'_, ClipboardHistory>) -> Vec<ClipboardEntry> {
    state.snapshot()
}

#[tauri::command]
pub async fn copy_clipboard_entry(
    state: tauri::State<'_, ClipboardHistory>,
    id: u64,
) -> Result<(), String> {
    let entry = state.get(id)?;
    match entry.kind {
        ClipboardKind::Text => {
            let text = entry.text.clone().unwrap_or_default();
            tauri::async_runtime::spawn_blocking(move || {
                let mut clipboard =
                    arboard::Clipboard::new().map_err(|error| format!("无法访问剪贴板：{error}"))?;
                clipboard
                    .set_text(text)
                    .map_err(|error| format!("无法写入剪贴板：{error}"))
            })
            .await
            .map_err(|error| format!("剪贴板回拷任务异常结束：{error}"))?
        }
        ClipboardKind::Image => {
            let path = state.image_path(id)?;
            tauri::async_runtime::spawn_blocking(move || {
                let image = load_image_data(&path)?;
                let mut clipboard =
                    arboard::Clipboard::new().map_err(|error| format!("无法访问剪贴板：{error}"))?;
                clipboard
                    .set_image(image)
                    .map_err(|error| format!("无法写入剪贴板：{error}"))
            })
            .await
            .map_err(|error| format!("剪贴板回拷任务异常结束：{error}"))?
        }
    }
}

#[tauri::command]
pub async fn get_clipboard_image(
    state: tauri::State<'_, ClipboardHistory>,
    id: u64,
) -> Result<String, String> {
    let entry = state.get(id)?;
    if entry.kind != ClipboardKind::Image {
        return Err("该记录不是图片".to_string());
    }
    let path = state.image_path(id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|error| format!("无法读取图片：{error}"))?;
        Ok::<_, String>(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    })
    .await
    .map_err(|error| format!("图片读取任务异常结束：{error}"))?
}

#[tauri::command]
pub fn set_clipboard_entry_pinned(
    app: AppHandle,
    state: tauri::State<'_, ClipboardHistory>,
    id: u64,
    pinned: bool,
) -> Result<(), String> {
    state.set_pinned(id, pinned)?;
    emit_snapshot(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn delete_clipboard_entry(
    app: AppHandle,
    state: tauri::State<'_, ClipboardHistory>,
    id: u64,
) -> Result<(), String> {
    state.delete(id)?;
    emit_snapshot(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn clear_clipboard_history(
    app: AppHandle,
    state: tauri::State<'_, ClipboardHistory>,
) -> Result<(), String> {
    state.clear();
    emit_snapshot(&app, &state);
    Ok(())
}

/// 把剪贴板里的一张图片作为「文件」拖拽给其他应用（Linux 走 GTK/Xdnd 的
/// `text/uri-list`，等价于从文件管理器里拖动一个图片文件）。
#[tauri::command]
pub fn drag_clipboard_image(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ClipboardHistory>,
    id: u64,
) -> Result<(), String> {
    let entry = state.get(id)?;
    if entry.kind != ClipboardKind::Image {
        return Err("该记录不是图片".to_string());
    }
    let path = state.image_path(id)?;
    let canonical = std::fs::canonicalize(&path).map_err(|error| format!("图片文件不可用：{error}"))?;

    #[cfg(target_os = "linux")]
    {
        use drag::{DragItem, DragMode, Image, Options};
        let panel_window = window.clone();
        let preview = drag_preview_png(&canonical);
        crate::background::mark_dragging(true);
        let result = drag::start_drag(
            &window.gtk_window().map_err(|error| format!("无法访问 GTK 窗口：{error}"))?,
            DragItem::Files(vec![canonical.clone()]),
            Image::Raw(preview),
            move |_result, _position| {
                crate::background::mark_dragging(false);
                if panel_window.label() == crate::panel::PANEL_LABEL {
                    let _ = panel_window.hide();
                }
            },
            Options {
                mode: DragMode::Copy,
                ..Options::default()
            },
        );
        if let Err(error) = result {
            crate::background::mark_dragging(false);
            return Err(format!("拖拽启动失败：{error}"));
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, canonical);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store {
            next_id: 1,
            entries: Vec::new(),
            data_path: None,
            image_dir: None,
        }
    }

    fn text_entry(id: u64, text: &str, char_count: usize, pinned: bool, captured_at_ms: u64) -> ClipboardEntry {
        ClipboardEntry {
            id,
            kind: ClipboardKind::Text,
            text: Some(text.to_string()),
            char_count: Some(char_count),
            pinned,
            captured_at_ms,
            content_hash: None,
            image_width: None,
            image_height: None,
            image_byte_count: None,
            image_preview: None,
        }
    }

    fn capture_text(store: &mut Store, text: &str) -> ClipboardEntry {
        store.capture_text(text).expect("capture should succeed")
    }

    #[test]
    fn trims_and_ignores_blank_text() {
        let mut store = store();
        assert!(store.capture_text("  \n ").is_none());
        capture_text(&mut store, "  hello  ");
        assert_eq!(store.entries[0].text.as_deref(), Some("hello"));
    }

    #[test]
    fn dedupes_by_moving_existing_entry_to_front() {
        let mut store = store();
        capture_text(&mut store, "one");
        capture_text(&mut store, "two");
        assert_eq!(store.entries.len(), 2);

        let moved = capture_text(&mut store, "one");
        assert_eq!(store.entries.len(), 2);
        assert_eq!(store.entries[0].text.as_deref(), Some("one"));
        assert_eq!(store.entries[1].text.as_deref(), Some("two"));
        assert_eq!(moved.id, store.entries[0].id);
        assert_eq!(moved.id, 1);
    }

    #[test]
    fn evicts_oldest_entries_beyond_cap() {
        let mut store = store();
        for index in 0..(MAX_ENTRIES + 5) {
            capture_text(&mut store, &format!("entry-{index}"));
        }
        assert_eq!(store.entries.len(), MAX_ENTRIES);
        let expected_first = format!("entry-{}", MAX_ENTRIES + 4);
        assert_eq!(store.entries[0].text.as_deref(), Some(expected_first.as_str()));
        assert_eq!(
            store.entries.last().and_then(|entry| entry.text.as_deref()),
            Some("entry-5")
        );
    }

    #[test]
    fn prefers_evicting_unpinned_entries() {
        let mut store = store();
        store.entries = vec![
            text_entry(1, "new", MAX_TOTAL_CHARS, false, 2),
            text_entry(2, "keep", 4, true, 1),
        ];
        store.enforce_caps();
        assert_eq!(store.entries.len(), 1);
        assert_eq!(store.entries[0].text.as_deref(), Some("keep"));
    }

    #[test]
    fn truncates_oversized_entries() {
        let mut store = store();
        capture_text(&mut store, &"y".repeat(MAX_ENTRY_CHARS + 100));
        assert!(store.entries[0].char_count.unwrap_or(0) <= MAX_ENTRY_CHARS + 1);
        assert!(store.entries[0].text.as_deref().unwrap_or("").ends_with('…'));
    }

    #[test]
    fn encodes_png_and_thumbnail_from_rgba() {
        // 2x2 RGBA（红、绿、蓝、白）。
        let rgba = [
            255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png = encode_png(&rgba, 2, 2).expect("png encode");
        assert!(png.len() > 8);
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 2));

        let data_url = thumbnail_data_url(&rgba, 2, 2).expect("thumbnail");
        assert!(data_url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn captures_dedupes_and_cleans_up_images() {
        let directory = std::env::temp_dir().join(format!("umanager-clip-test-{}", now_ms()));
        let mut store = Store {
            next_id: 1,
            entries: Vec::new(),
            data_path: None,
            image_dir: Some(directory.clone()),
        };
        let rgba = [
            255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];

        let entry = store.capture_image(&rgba, 2, 2).expect("capture image");
        assert_eq!(entry.kind, ClipboardKind::Image);
        assert!(store.image_path(entry.id).unwrap().exists());

        let again = store.capture_image(&rgba, 2, 2).expect("capture duplicate");
        assert_eq!(again.id, entry.id);
        assert_eq!(store.entries.len(), 1);

        store.delete(entry.id).unwrap();
        assert!(store.entries.is_empty());
        assert!(!directory.join(format!("{}.png", entry.id)).exists());

        let _ = std::fs::remove_dir_all(&directory);
    }
}
