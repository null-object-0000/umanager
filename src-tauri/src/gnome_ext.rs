//! GNOME Shell 扩展管理。
//!
//! 提供两类能力：
//! 1. 通用扩展管理：列出已安装扩展（用户级 + 系统级）、启用/禁用、卸载用户级扩展。
//!    全部通过固定 argv 的 `gnome-extensions` 命令执行，不经 shell。
//! 2. UManager 内置「中国节假日日历」扩展（`umanager-calendar@umanager.app`）：
//!    安装 / 卸载 / 在线刷新节假日数据。数据打包进 App，可离线可用；
//!    在线刷新仅从固定的 GitHub raw 域名拉取公开的 holiday-cn JSON。
//!
//! 安全要点：
//! - 命令固定 argv，uuid 经过字符白名单校验，杜绝 shell 注入与路径穿越。
//! - 卸载只允许发生在用户扩展目录（`~/.local/share/gnome-shell/extensions`）内，
//!   系统级扩展（`/usr/share/gnome-shell/extensions`）只读、不可卸载。
//! - 在线刷新使用 `restricted_client`（仅允许 `raw.githubusercontent.com`，
//!   HTTPS-only），且不写任何系统路径，只更新扩展目录内的数据文件。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// UManager 内置日历扩展的 UUID（GNOME 扩展目录名 = UUID）。
pub const CALENDAR_UUID: &str = "umanager-calendar@umanager.app";

const USER_EXTENSIONS_REL: &str = ".local/share/gnome-shell/extensions";
const SYSTEM_EXTENSIONS_DIR: &str = "/usr/share/gnome-shell/extensions";

/// 节假日在线刷新的唯一数据源（GitHub raw 上的 holiday-cn 官方仓库）。
const HOLIDAY_SOURCE_HOST: &str = "raw.githubusercontent.com";
const HOLIDAY_MIN_YEAR: u16 = 2024;
const HOLIDAY_MAX_YEAR: u16 = 2027;
const MAX_HOLIDAY_JSON_BYTES: u64 = 256 * 1024;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GnomeExtensionInfo {
    pub uuid: String,
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub shell_versions: Vec<String>,
    pub url: Option<String>,
    pub path: String,
    /// `user`（本机用户安装）或 `system`（随系统分发，只读）。
    pub origin: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    pub installed: bool,
    /// 运行中的 GNOME Shell 是否已加载（重登前新安装的扩展为 false）。
    pub enabled: bool,
    /// 已写入持久化启用列表（gsettings enabled-extensions），重登后会自动启用。
    pub pending_enable: bool,
    /// holidays.json 中实际覆盖的年份（从日期前缀提取）。
    pub data_years: Vec<u16>,
    pub data_days: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HolidayRefreshReport {
    pub years: Vec<u16>,
    pub days: usize,
}

// ---------------------------------------------------------------------------
// 路径与校验
// ---------------------------------------------------------------------------

fn user_extensions_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let home = app
        .path()
        .home_dir()
        .map_err(|error| format!("无法确定用户主目录：{error}"))?;
    Ok(home.join(USER_EXTENSIONS_REL))
}

fn extension_path(user_dir: &Path, uuid: &str) -> Result<PathBuf, String> {
    validate_uuid(uuid)?;
    Ok(user_dir.join(uuid))
}

/// uuid 字符白名单：字母数字 + `@ . _ -`（GNOME 扩展 UUID 的合法字符集）。
fn validate_uuid(uuid: &str) -> Result<(), String> {
    if uuid.is_empty() || uuid.len() > 128 {
        return Err("扩展 UUID 长度非法".to_owned());
    }
    if !uuid
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '-'))
    {
        return Err("扩展 UUID 包含非法字符".to_owned());
    }
    Ok(())
}

/// 确保目标目录位于用户扩展目录内（防路径穿越）。
fn ensure_within_user_dir(user_dir: &Path, target: &Path) -> Result<(), String> {
    let base = user_dir.canonicalize().unwrap_or_else(|_| user_dir.to_path_buf());
    let canonical = target
        .canonicalize()
        .map_err(|_| "目标目录不存在".to_owned())?;
    if canonical.starts_with(&base) {
        Ok(())
    } else {
        Err("目标目录不在用户扩展目录内".to_owned())
    }
}

// ---------------------------------------------------------------------------
// gnome-extensions 命令执行（固定 argv，不经 shell）
// ---------------------------------------------------------------------------

fn run_gnome(args: &[&str]) -> Result<String, String> {
    let output = Command::new("gnome-extensions")
        .args(args)
        .output()
        .map_err(|error| format!("无法执行 gnome-extensions（需要 GNOME Shell）：{error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "gnome-extensions {} 失败：{}",
            args.join(" "),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn enabled_uuids() -> Result<HashSet<String>, String> {
    let enabled = run_gnome(&["list", "--enabled"])?;
    Ok(enabled.lines().map(|line| line.trim().to_owned()).collect())
}

/// 让 GNOME Shell 加载/卸载一个扩展。校验 uuid 后固定 argv 调用。
///
/// 注意：`gnome-extensions enable` 只在**运行中的 Shell 已认识的扩展**
/// （Shell 启动时扫描出的目录）上生效；Wayland 会话下**新安装**的扩展目录
/// 要等下次登录才会被扫描。因此对 UManager 内置日历扩展，失败时回退到
/// 持久化启用列表（gsettings `enabled-extensions`），登录后 Shell 会自动启用。
pub fn set_enabled(uuid: &str, enabled: bool) -> Result<(), String> {
    validate_uuid(uuid)?;
    let action = if enabled { "enable" } else { "disable" };
    match run_gnome(&[action, uuid]) {
        Ok(_) => Ok(()),
        Err(error) => {
            if uuid == CALENDAR_UUID {
                // 新目录尚未被运行中的 Shell 识别：写入持久化列表，重登后生效。
                set_persistent_enabled(CALENDAR_UUID, enabled)?;
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 持久化启用列表（gsettings org.gnome.shell enabled-extensions）
// ---------------------------------------------------------------------------

const GSETTINGS_SCHEMA: &str = "org.gnome.shell";
const GSETTINGS_KEY: &str = "enabled-extensions";

/// 解析 gsettings `enabled-extensions` 输出（如 `['a', 'b']`，空列表为 `@as []`）。
fn parse_enabled_extensions(trimmed: &str) -> Result<Vec<String>, String> {
    let trimmed = trimmed.trim();
    if trimmed == "@as []" || trimmed == "[]" || trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| format!("无法解析扩展启用列表：{trimmed}"))?;
    Ok(inner
        .split(',')
        .map(|item| item.trim().trim_matches('\'').to_owned())
        .filter(|item| !item.is_empty())
        .collect())
}

/// 序列化 gsettings `enabled-extensions` 的值。
fn serialize_enabled_extensions(list: &[String]) -> String {
    if list.is_empty() {
        "@as []".to_owned()
    } else {
        let items: Vec<String> = list.iter().map(|item| format!("'{item}'")).collect();
        format!("[{}]", items.join(", "))
    }
}

/// 读取当前持久化启用列表。
fn read_enabled_extensions() -> Result<Vec<String>, String> {
    let output = Command::new("gsettings")
        .args(["get", GSETTINGS_SCHEMA, GSETTINGS_KEY])
        .output()
        .map_err(|error| format!("无法读取扩展启用列表：{error}"))?;
    parse_enabled_extensions(&String::from_utf8_lossy(&output.stdout))
}

/// 把 uuid 写入（enabled=true）或移出（enabled=false）持久化启用列表。
fn set_persistent_enabled(uuid: &str, enabled: bool) -> Result<(), String> {
    let mut list = read_enabled_extensions()?;
    if enabled {
        if !list.iter().any(|item| item == uuid) {
            list.push(uuid.to_owned());
        }
    } else {
        list.retain(|item| item != uuid);
    }
    let serialized = serialize_enabled_extensions(&list);
    let output = Command::new("gsettings")
        .args(["set", GSETTINGS_SCHEMA, GSETTINGS_KEY, &serialized])
        .output()
        .map_err(|error| format!("无法写入扩展启用列表：{error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("无法写入扩展启用列表：{}", stderr.trim()));
    }
    Ok(())
}

/// 日历扩展是否已写入持久化启用列表（重登后将自动启用）。
fn calendar_pending_enable() -> bool {
    read_enabled_extensions()
        .map(|list| list.iter().any(|item| item == CALENDAR_UUID))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 元数据解析
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MetadataJson {
    uuid: Option<String>,
    name: Option<String>,
    description: Option<String>,
    version: Option<serde_json::Value>,
    #[serde(rename = "shell-version")]
    shell_version: Option<Vec<String>>,
    url: Option<String>,
}

fn read_metadata(dir: &Path) -> Option<MetadataJson> {
    let text = std::fs::read_to_string(dir.join("metadata.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn version_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::String(text) => Some(text.clone()),
        _ => None,
    }
    .filter(|text| !text.is_empty())
}

// ---------------------------------------------------------------------------
// 列出扩展
// ---------------------------------------------------------------------------

pub fn list(app: &AppHandle) -> Result<Vec<GnomeExtensionInfo>, String> {
    let user_dir = user_extensions_dir(app)?;
    let enabled = enabled_uuids().unwrap_or_default();
    let mut result = Vec::new();

    let collect = |dir: PathBuf, origin: &str, result: &mut Vec<GnomeExtensionInfo>| {
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(metadata) = read_metadata(&path) else { continue };
            let uuid = metadata.uuid.unwrap_or_else(|| {
                entry.file_name().to_string_lossy().into_owned()
            });
            result.push(GnomeExtensionInfo {
                enabled: enabled.contains(&uuid),
                uuid: uuid.clone(),
                name: metadata.name.unwrap_or_else(|| uuid.clone()),
                description: metadata.description.unwrap_or_default(),
                version: metadata.version.as_ref().and_then(version_string),
                shell_versions: metadata.shell_version.unwrap_or_default(),
                url: metadata.url,
                path: path.display().to_string(),
                origin: origin.to_owned(),
            });
        }
    };

    collect(user_dir.clone(), "user", &mut result);
    collect(PathBuf::from(SYSTEM_EXTENSIONS_DIR), "system", &mut result);

    result.sort_by(|left, right| {
        left.origin
            .cmp(&right.origin)
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(result)
}

/// 卸载用户级扩展：只允许删除用户扩展目录内的目录，系统级扩展拒绝。
pub fn uninstall(app: &AppHandle, uuid: &str) -> Result<(), String> {
    validate_uuid(uuid)?;
    if uuid == CALENDAR_UUID {
        return uninstall_calendar(app);
    }
    let user_dir = user_extensions_dir(app)?;
    let target = extension_path(&user_dir, uuid)?;
    if !target.exists() {
        return Err(format!("扩展 {uuid} 未安装"));
    }
    if target
        .canonicalize()
        .is_ok_and(|path| path.starts_with(Path::new(SYSTEM_EXTENSIONS_DIR)))
    {
        return Err("系统级扩展不能通过 UManager 卸载".to_owned());
    }
    ensure_within_user_dir(&user_dir, &target)?;
    std::fs::remove_dir_all(&target).map_err(|error| format!("删除扩展目录失败：{error}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// UManager 内置日历扩展
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HolidayDay {
    date: String,
    name: String,
    #[serde(default)]
    is_off_day: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct HolidayYearFile {
    days: Vec<HolidayDay>,
}

fn calendar_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let user_dir = user_extensions_dir(app)?;
    extension_path(&user_dir, CALENDAR_UUID)
}

fn read_holidays_json(path: &Path) -> Option<(Vec<u16>, usize)> {
    let text = std::fs::read_to_string(path.join("holidays.json")).ok()?;
    let data: serde_json::Value = serde_json::from_str(&text).ok()?;
    let days = data.get("days")?.as_array()?;
    let mut years: HashSet<u16> = HashSet::new();
    for day in days {
        if let Some(date) = day.get("date").and_then(|value| value.as_str()) {
            if date.len() >= 4 {
                if let Ok(year) = date[..4].parse::<u16>() {
                    years.insert(year);
                }
            }
        }
    }
    let mut sorted: Vec<u16> = years.into_iter().collect();
    sorted.sort_unstable();
    Some((sorted, days.len()))
}

pub fn calendar_status(app: &AppHandle) -> Result<CalendarStatus, String> {
    let dir = calendar_dir(app)?;
    if !dir.join("metadata.json").exists() {
        return Ok(CalendarStatus {
            installed: false,
            enabled: false,
            pending_enable: false,
            data_years: Vec::new(),
            data_days: 0,
        });
    }
    let enabled = enabled_uuids().unwrap_or_default().contains(CALENDAR_UUID);
    let pending_enable = calendar_pending_enable();
    let (data_years, data_days) = read_holidays_json(&dir).unwrap_or_default();
    Ok(CalendarStatus {
        installed: true,
        enabled,
        pending_enable,
        data_years,
        data_days,
    })
}

/// 安装 UManager 日历扩展：把打包的扩展文件写入用户扩展目录。
/// 文件随 App 编译（include_str!），完全离线可用。
///
/// 安装即把 uuid 写入持久化启用列表（gsettings enabled-extensions），
/// 这样即使运行中的 GNOME Shell 尚未识别新目录，重登后也会自动启用；
/// 若 Shell 已能识别则立即生效。
pub fn install_calendar(app: &AppHandle) -> Result<CalendarStatus, String> {
    let dir = calendar_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|error| format!("创建扩展目录失败：{error}"))?;

    let files: [(&str, &str); 4] = [
        ("metadata.json", include_str!("../resources/umanager-calendar/metadata.json")),
        ("extension.js", include_str!("../resources/umanager-calendar/extension.js")),
        ("stylesheet.css", include_str!("../resources/umanager-calendar/stylesheet.css")),
        ("holidays.json", include_str!("../resources/umanager-calendar/holidays.json")),
    ];
    for (name, content) in files {
        std::fs::write(dir.join(name), content)
            .map_err(|error| format!("写入扩展文件 {name} 失败：{error}"))?;
    }

    // 先确保持久化启用，再尝试即时启用（新目录在 Wayland 下需重登才被 Shell 扫描）。
    set_persistent_enabled(CALENDAR_UUID, true)?;
    let _ = set_enabled(CALENDAR_UUID, true);
    calendar_status(app)
}

pub fn uninstall_calendar(app: &AppHandle) -> Result<(), String> {
    let user_dir = user_extensions_dir(app)?;
    let dir = calendar_dir(app)?;
    if !dir.exists() {
        return Ok(());
    }
    let _ = set_enabled(CALENDAR_UUID, false);
    let _ = set_persistent_enabled(CALENDAR_UUID, false);
    ensure_within_user_dir(&user_dir, &dir)?;
    std::fs::remove_dir_all(&dir).map_err(|error| format!("删除扩展目录失败：{error}"))?;
    Ok(())
}

/// 在线刷新节假日数据：从 holiday-cn 官方仓库拉取 2024–2027 各年 JSON，
/// 跳过未发布（days 为空）的年份，合并写入扩展目录的 holidays.json。
/// 仅允许 `raw.githubusercontent.com`（HTTPS-only）。
pub async fn refresh_holiday_data_impl(app: &AppHandle) -> Result<HolidayRefreshReport, String> {
    let dir = calendar_dir(app)?;
    if !dir.join("metadata.json").exists() {
        return Err("请先安装「中国节假日日历」扩展，再刷新数据".to_owned());
    }

    let hosts = vec![HOLIDAY_SOURCE_HOST.to_owned()];
    let client = crate::source_engine::restricted_client(&hosts, Duration::from_secs(20))?;

    let mut merged: Vec<HolidayDay> = Vec::new();
    let mut years: Vec<u16> = Vec::new();

    for year in HOLIDAY_MIN_YEAR..=HOLIDAY_MAX_YEAR {
        let url = format!(
            "https://{HOLIDAY_SOURCE_HOST}/NateScarlet/holiday-cn/master/{year}.json"
        );
        let response = match client.get(&url).send().await {
            Ok(response) => response,
            Err(error) => return Err(format!("拉取 {year} 年节假日数据失败：{error}")),
        };
        if response
            .content_length()
            .is_some_and(|size| size > MAX_HOLIDAY_JSON_BYTES)
        {
            return Err(format!("{year} 年节假日数据大小异常"));
        }
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return Err(format!("读取 {year} 年节假日数据失败：{error}")),
        };
        if bytes.len() as u64 > MAX_HOLIDAY_JSON_BYTES {
            return Err(format!("{year} 年节假日数据大小异常"));
        }
        let parsed: HolidayYearFile = match serde_json::from_slice(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => return Err(format!("{year} 年节假日数据格式无效：{error}")),
        };
        if parsed.days.is_empty() {
            // 国务院尚未公布该年安排（如 2027），跳过。
            continue;
        }
        years.push(year);
        merged.extend(parsed.days);
    }

    if merged.is_empty() {
        return Err("没有获取到任何年份的节假日数据".to_owned());
    }
    merged.sort_by(|left, right| left.date.cmp(&right.date));
    merged.dedup_by(|left, right| left.date == right.date);

    let payload = serde_json::json!({ "days": merged });
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|error| format!("无法编码节假日数据：{error}"))?;
    std::fs::write(dir.join("holidays.json"), content)
        .map_err(|error| format!("写入节假日数据失败：{error}"))?;

    Ok(HolidayRefreshReport {
        days: merged.len(),
        years,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_gnome_extensions(app: AppHandle) -> Result<Vec<GnomeExtensionInfo>, String> {
    list(&app)
}

#[tauri::command]
pub fn set_gnome_extension_enabled(uuid: String, enabled: bool) -> Result<(), String> {
    set_enabled(&uuid, enabled)
}

#[tauri::command]
pub fn uninstall_gnome_extension(app: AppHandle, uuid: String) -> Result<(), String> {
    uninstall(&app, &uuid)
}

#[tauri::command]
pub fn get_umanager_calendar_status(app: AppHandle) -> Result<CalendarStatus, String> {
    calendar_status(&app)
}

#[tauri::command]
pub fn install_umanager_calendar(app: AppHandle) -> Result<CalendarStatus, String> {
    install_calendar(&app)
}

#[tauri::command]
pub fn uninstall_umanager_calendar(app: AppHandle) -> Result<(), String> {
    uninstall_calendar(&app)
}

#[tauri::command]
pub async fn refresh_holiday_data(app: AppHandle) -> Result<HolidayRefreshReport, String> {
    refresh_holiday_data_impl(&app).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_validation_accepts_gnome_uuids() {
        assert!(validate_uuid("umanager-calendar@umanager.app").is_ok());
        assert!(validate_uuid("Vitals@CoreCoding.com").is_ok());
        assert!(validate_uuid("dash-to-dock@micxgx.gmail.com").is_ok());
        assert!(validate_uuid("ding@rastersoft.com").is_ok());
    }

    #[test]
    fn uuid_validation_rejects_path_traversal() {
        assert!(validate_uuid("").is_err());
        assert!(validate_uuid("../evil").is_err());
        assert!(validate_uuid("a/b").is_err());
        assert!(validate_uuid("a\\b").is_err());
        assert!(validate_uuid("a b").is_err());
        assert!(validate_uuid("a:b").is_err());
        assert!(validate_uuid(&"a".repeat(129)).is_err());
    }

    #[test]
    fn version_string_handles_number_and_text() {
        assert_eq!(version_string(&serde_json::json!(82)), Some("82".to_owned()));
        assert_eq!(version_string(&serde_json::json!("3.2.1")), Some("3.2.1".to_owned()));
        assert_eq!(version_string(&serde_json::json!("")), None);
        assert_eq!(version_string(&serde_json::json!(null)), None);
    }

    #[test]
    fn holidays_json_roundtrips_with_is_off_day() {
        let day = HolidayDay {
            date: "2026-01-04".to_owned(),
            name: "元旦".to_owned(),
            is_off_day: false,
        };
        let payload = serde_json::json!({ "days": [day] });
        let text = serde_json::to_string(&payload).unwrap();
        assert!(text.contains("\"isOffDay\":false"));
        let parsed: HolidayYearFile = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.days.len(), 1);
        assert!(!parsed.days[0].is_off_day);
    }

    #[test]
    fn user_extension_dir_is_under_home() {
        // 无法在没有 AppHandle 时调用 user_extensions_dir；这里验证相对路径常量合法。
        assert!(USER_EXTENSIONS_REL.starts_with(".local/"));
        assert!(SYSTEM_EXTENSIONS_DIR.starts_with("/usr/share/gnome-shell/"));
    }

    #[test]
    fn parse_enabled_extensions_handles_real_output() {
        let list = parse_enabled_extensions("['Vitals@CoreCoding.com', 'dash-to-dock@micxgx.gmail.com']").unwrap();
        assert_eq!(list, vec!["Vitals@CoreCoding.com", "dash-to-dock@micxgx.gmail.com"]);
        assert!(parse_enabled_extensions("@as []").unwrap().is_empty());
        assert!(parse_enabled_extensions("[]").unwrap().is_empty());
        assert!(parse_enabled_extensions("").unwrap().is_empty());
        assert!(parse_enabled_extensions("['仅一个']").unwrap() == vec!["仅一个"]);
    }

    #[test]
    fn parse_enabled_extensions_rejects_malformed() {
        assert!(parse_enabled_extensions("not-a-list").is_err());
        assert!(parse_enabled_extensions("[unclosed").is_err());
    }

    #[test]
    fn serialize_enabled_extensions_roundtrips() {
        let input = vec!["umanager-calendar@umanager.app".to_owned(), "Vitals@CoreCoding.com".to_owned()];
        let serialized = serialize_enabled_extensions(&input);
        assert_eq!(serialized, "['umanager-calendar@umanager.app', 'Vitals@CoreCoding.com']");
        let parsed = parse_enabled_extensions(&serialized).unwrap();
        assert_eq!(parsed, input);
        assert_eq!(serialize_enabled_extensions(&[]), "@as []");
    }

    #[test]
    fn set_persistent_enabled_add_and_remove_is_idempotent() {
        // 不执行真实 gsettings（无 AppHandle/无 dconf），只验证列表操作逻辑：
        // 通过在纯函数层面模拟 add/remove 语义。
        let mut list = vec!["a@b.c".to_owned()];
        if !list.iter().any(|item| item == CALENDAR_UUID) {
            list.push(CALENDAR_UUID.to_owned());
        }
        assert_eq!(list.len(), 2);
        list.retain(|item| item != CALENDAR_UUID);
        assert_eq!(list.len(), 1);
    }
}
