//! GNOME Shell 扩展管理。
//!
//! 提供通用扩展管理能力：列出已安装扩展（用户级 + 系统级）、启用/禁用、
//! 卸载用户级扩展。全部通过固定 argv 的 `gnome-extensions` 命令执行，不经 shell。
//!
//! 安全要点：
//! - 命令固定 argv，uuid 经过字符白名单校验，杜绝 shell 注入与路径穿越。
//! - 卸载只允许发生在用户扩展目录（`~/.local/share/gnome-shell/extensions`）内，
//!   系统级扩展（`/usr/share/gnome-shell/extensions`）只读、不可卸载。
//! - **不再内置任何具体扩展**（如中国节假日日历已拆为独立仓库
//!   `holiday-calendar-cn`，由用户自行安装）；这里只管理任意已安装的扩展。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager};

const USER_EXTENSIONS_REL: &str = ".local/share/gnome-shell/extensions";
const SYSTEM_EXTENSIONS_DIR: &str = "/usr/share/gnome-shell/extensions";

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
/// 要等下次登录才会被扫描。因此当 enable/disable 对"新装扩展"失败时，
/// 回退到持久化启用列表（gsettings `enabled-extensions`），登录后 Shell 会自动启用。
pub fn set_enabled(uuid: &str, enabled: bool) -> Result<(), String> {
    validate_uuid(uuid)?;
    let action = if enabled { "enable" } else { "disable" };
    match run_gnome(&[action, uuid]) {
        Ok(_) => Ok(()),
        Err(_error) => {
            // 新装目录尚未被运行中的 Shell 识别：写入持久化列表，重登后生效。
            set_persistent_enabled(uuid, enabled)?;
            Ok(())
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
    let user_dir = user_extensions_dir(app)?;
    let target = extension_path(&user_dir, uuid)?;
    if !target.exists() {
        return Err(format!("扩展 {uuid} 未安装"));
    }
    if target.canonicalize().is_ok_and(|path| path.starts_with(Path::new(SYSTEM_EXTENSIONS_DIR))) {
        return Err("系统级扩展不能通过 UManager 卸载".to_owned());
    }
    ensure_within_user_dir(&user_dir, &target)?;
    std::fs::remove_dir_all(&target).map_err(|error| format!("删除扩展目录失败：{error}"))?;
    Ok(())
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_validation_accepts_gnome_uuids() {
        assert!(validate_uuid("holiday-calendar-cn@github.io").is_ok());
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
        let input = vec!["holiday-calendar-cn@github.io".to_owned(), "Vitals@CoreCoding.com".to_owned()];
        let serialized = serialize_enabled_extensions(&input);
        assert_eq!(serialized, "['holiday-calendar-cn@github.io', 'Vitals@CoreCoding.com']");
        let parsed = parse_enabled_extensions(&serialized).unwrap();
        assert_eq!(parsed, input);
        assert_eq!(serialize_enabled_extensions(&[]), "@as []");
    }

    #[test]
    fn user_extension_dir_is_under_home() {
        assert!(USER_EXTENSIONS_REL.starts_with(".local/"));
        assert!(SYSTEM_EXTENSIONS_DIR.starts_with("/usr/share/gnome-shell/"));
    }
}
