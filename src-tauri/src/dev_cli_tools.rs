use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Manager};
use umanager_catalog::{Catalog, DevToolInstaller, DevToolUninstall, DevToolUpdate, DevelopmentTool};

const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
const MAX_LOG_LINE_CHARS: usize = 2_000;

/// Per-tool version-line selections persisted in the app config dir (e.g. a
/// user who switched dsh to the `alpha` line). Keyed by tool id; values are
/// npm dist-tag names. Absent tools fall back to their configured `distTag`.
const CHANNEL_CONFIG_FILE_NAME: &str = "tool-channels.json";

static CHANNEL_SELECTIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn channel_selections_lock() -> &'static Mutex<HashMap<String, String>> {
    CHANNEL_SELECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Where the current user keeps officially-installed CLI binaries. These are the
/// locations used by the vendor installers configured in `vendors.json`.
fn known_binary_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local").join("bin"),
        home.join(".opencode").join("bin"),
        home.join(".npm-global").join("bin"),
        // pnpm 官方安装脚本（get.pnpm.io）默认把二进制放进 ~/.local/share/pnpm/bin，
        // 该目录不在桌面进程的 PATH 中，需显式登记才能被检测。
        home.join(".local").join("share").join("pnpm").join("bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ]
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevToolState {
    pub tool_id: String,
    pub display_name: String,
    pub vendor: String,
    pub homepage: String,
    pub icon: Option<String>,
    pub accent_color: Option<String>,
    pub binary_name: String,
    pub npm_package: Option<String>,
    /// `npm` or `curlScript`, mirroring the configured installer.
    pub installer_kind: String,
    pub npm_available: bool,
    pub installed: bool,
    /// `npmGlobal`, `officialInstaller`, `onPath` or `null` when not installed.
    pub install_kind: Option<String>,
    pub version: Option<String>,
    pub latest_version: Option<String>,
    /// Every npm dist-tag channel from the signed feed (tag -> version). `None`
    /// for non-npm tools and older feeds without channel data.
    pub channels: Option<BTreeMap<String, String>>,
    /// The version line in effect: the user's persisted selection when it is
    /// still offered by the feed, otherwise the tool's configured `distTag`.
    pub selected_channel: Option<String>,
    pub binary_path: Option<String>,
    pub update_available: bool,
    pub can_uninstall: bool,
    /// Markdown changelog for the latest version, from the signed feed.
    pub release_notes: Option<String>,
    /// HTTPS link to the canonical changelog page.
    pub release_notes_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevToolProgress {
    pub tool_id: String,
    pub phase: &'static str,
    pub stream: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevToolReport {
    pub tool_id: String,
    pub action: String,
    pub success: bool,
    pub message: String,
}

pub type DevToolProgressCallback = Arc<dyn Fn(DevToolProgress) + Send + Sync>;

pub fn load_tools() -> Result<Vec<DevelopmentTool>, String> {
    Ok(Catalog::load()?.development_tools)
}

pub async fn detect_state(tool_id: String) -> Result<DevToolState, String> {
    let tool = tool_by_id(&tool_id)?;
    let feed_entry = feed_tool_entry(&tool).await;
    tauri::async_runtime::spawn_blocking(move || detect_state_sync(&tool, feed_entry))
        .await
        .map_err(|error| format!("命令行工具检测任务异常结束：{error}"))?
}

/// The signed feed's entry for a tool, best-effort. The npm package is
/// cross-checked so a stale feed entry for a renamed package is never trusted.
async fn feed_tool_entry(tool: &DevelopmentTool) -> Option<crate::feed::FeedToolEntry> {
    let lookup_package = tool.npm_package.clone();
    crate::feed::tool_entry(&tool.tool_id)
        .await
        .ok()
        .flatten()
        .filter(|entry| entry.npm_package == lookup_package)
}

/// The exact version to install for the tool's selected version line: the
/// selected channel's version from the signed feed when channels are offered,
/// otherwise the feed's resolved `version` (the configured channel). `None`
/// only when the feed is unavailable — callers then fall back to installing
/// the configured dist-tag, the pre-feed behavior.
fn target_version_for(
    feed_entry: Option<&crate::feed::FeedToolEntry>,
    persisted_selection: Option<&str>,
) -> Option<String> {
    let entry = feed_entry?;
    if let Some(channels) = &entry.channels {
        if let Some(channel) = persisted_selection
            && let Some(version) = channels.get(channel)
        {
            return Some(version.clone());
        }
    }
    Some(entry.version.clone())
}

pub async fn install(
    tool_id: String,
    progress: DevToolProgressCallback,
) -> Result<DevToolReport, String> {
    let tool = tool_by_id(&tool_id)?;
    // Install the exact version the signed feed advertises for the selected
    // version line, so the installed version always matches what the UI shows.
    let persisted_channel = selected_channel(&tool.tool_id);
    let target_version =
        target_version_for(feed_tool_entry(&tool).await.as_ref(), persisted_channel.as_deref());
    tauri::async_runtime::spawn_blocking(move || {
        let home = user_home()?;
        let mut command = install_command(&tool, &home, target_version.as_deref())?;
        let output = run_streaming(&mut command, &tool.tool_id, &format!("开始安装 {}（{}）", tool.display_name, installer_label(&tool)), Some(&progress))?;
        Ok(DevToolReport {
            tool_id: tool.tool_id.clone(),
            action: "install".to_owned(),
            success: true,
            message: tail_summary(&output),
        })
    })
    .await
    .map_err(|error| format!("命令行工具安装任务异常结束：{error}"))?
}

pub async fn update(
    tool_id: String,
    progress: DevToolProgressCallback,
) -> Result<DevToolReport, String> {
    let tool = tool_by_id(&tool_id)?;
    let persisted_channel = selected_channel(&tool.tool_id);
    let target_version =
        target_version_for(feed_tool_entry(&tool).await.as_ref(), persisted_channel.as_deref());
    tauri::async_runtime::spawn_blocking(move || {
        let home = user_home()?;
        let (mut command, label) = update_command(&tool, &home, target_version.as_deref())?;
        let output = run_streaming(&mut command, &tool.tool_id, &format!("开始更新 {}（{}）", tool.display_name, label), Some(&progress))?;
        Ok(DevToolReport {
            tool_id: tool.tool_id.clone(),
            action: "update".to_owned(),
            success: true,
            message: tail_summary(&output),
        })
    })
    .await
    .map_err(|error| format!("命令行工具更新任务异常结束：{error}"))?
}

pub async fn uninstall(
    tool_id: String,
    progress: DevToolProgressCallback,
) -> Result<DevToolReport, String> {
    let tool = tool_by_id(&tool_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let home = user_home()?;
        let state = detect_state_sync(&tool, None)?;
        let install_kind = state
            .install_kind
            .ok_or_else(|| format!("未检测到已安装的 {}", tool.display_name))?;
        let mut command = uninstall_command(&tool, &home, &install_kind)?;
        let output = run_streaming(&mut command, &tool.tool_id, &format!("开始卸载 {}（{}）", tool.display_name, installer_label(&tool)), Some(&progress))?;
        Ok(DevToolReport {
            tool_id: tool.tool_id.clone(),
            action: "uninstall".to_owned(),
            success: true,
            message: tail_summary(&output),
        })
    })
    .await
    .map_err(|error| format!("命令行工具卸载任务异常结束：{error}"))?
}

fn tool_by_id(tool_id: &str) -> Result<DevelopmentTool, String> {
    Catalog::load()?
        .by_tool_id(tool_id)
        .cloned()
        .ok_or_else(|| format!("软件源中不存在命令行工具 {tool_id}"))
}

/// Release notes to show for a tool: the notes of the selected version line
/// (looked up by version in `channelReleaseNotes`) when the feed carries them,
/// otherwise the entry-level notes for the default line.
fn release_notes_for(
    feed_entry: Option<&crate::feed::FeedToolEntry>,
    selected_version: Option<&str>,
) -> (Option<String>, Option<String>) {
    if let Some(entry) = feed_entry
        && let Some(version) = selected_version
        && let Some(channel_notes) = &entry.channel_release_notes
        && let Some(notes) = channel_notes.get(version)
    {
        return (notes.release_notes.clone(), notes.release_notes_url.clone());
    }
    (
        feed_entry.and_then(|entry| entry.release_notes.clone()),
        feed_entry.and_then(|entry| entry.release_notes_url.clone()),
    )
}

fn detect_state_sync(tool: &DevelopmentTool, feed_entry: Option<crate::feed::FeedToolEntry>) -> Result<DevToolState, String> {
    let home = user_home()?;
    let npm_available = npm_available(&home);
    // Latest versions come exclusively from the central metadata feed; npm stays
    // available for install/uninstall, not for version lookups.
    let channels = feed_entry.as_ref().and_then(|entry| entry.channels.clone());
    let selected_channel =
        effective_channel(tool, channels.as_ref(), selected_channel(&tool.tool_id).as_deref());
    let latest_version = feed_entry.as_ref().map(|entry| {
        // The selected version line's version when the feed offers channels,
        // otherwise the configured channel's resolved version.
        match (&selected_channel, entry.channels.as_ref()) {
            (Some(channel), Some(channels)) => {
                channels.get(channel).cloned().unwrap_or_else(|| entry.version.clone())
            }
            _ => entry.version.clone(),
        }
    });

    let binary = find_tool_binary(tool, &home);
    let install_kind = binary
        .as_ref()
        .map(|path| classify_install_kind(tool, &home, path, npm_available));
    let version = binary
        .as_ref()
        .and_then(|path| capture_version(path))
        .or_else(|| {
            if install_kind.as_deref() == Some("npmGlobal") && npm_available {
                tool.npm_package
                    .as_deref()
                    .and_then(|package| npm_installed_version(&home, package))
            } else {
                None
            }
        });

    let update_available = match (&version, &latest_version) {
        (Some(installed), Some(latest)) => compare_versions(installed, latest) == Ordering::Less,
        _ => false,
    };
    let can_uninstall = matches!(
        install_kind.as_deref(),
        Some("npmGlobal") | Some("officialInstaller")
    );
    let (release_notes, release_notes_url) =
        release_notes_for(feed_entry.as_ref(), latest_version.as_deref());

    Ok(DevToolState {
        tool_id: tool.tool_id.clone(),
        display_name: tool.display_name.clone(),
        vendor: tool.vendor.clone(),
        homepage: tool.homepage.clone(),
        icon: tool.icon.clone(),
        accent_color: tool.accent_color.clone(),
        binary_name: tool.binary_name.clone(),
        npm_package: tool.npm_package.clone(),
        installer_kind: match &tool.installer {
            DevToolInstaller::Npm => "npm".to_owned(),
            DevToolInstaller::CurlScript { .. } => "curlScript".to_owned(),
        },
        npm_available,
        installed: binary.is_some() || install_kind.as_deref() == Some("npmGlobal"),
        install_kind,
        version,
        latest_version,
        channels,
        selected_channel,
        binary_path: binary.map(|path| path.to_string_lossy().into_owned()),
        update_available,
        can_uninstall,
        release_notes,
        release_notes_url,
    })
}

/// The version line in effect for a tool: the user's persisted selection when
/// the feed still offers it, otherwise the tool's configured dist-tag channel
/// (`distTag` in vendors.json, defaulting to `latest`). When the feed offers
/// channels but neither the selection nor the configured tag is among them
/// (stale config), fall back to the conventional `latest` line, then any
/// offered channel — so the UI always shows a real line.
fn effective_channel(
    tool: &DevelopmentTool,
    channels: Option<&BTreeMap<String, String>>,
    persisted_selection: Option<&str>,
) -> Option<String> {
    if let Some(channel) = persisted_selection
        && let Some(channels) = channels
        && channels.contains_key(channel)
    {
        return Some(channel.to_owned());
    }
    if let Some(channels) = channels {
        if let Some(tag) = tool.dist_tag.as_deref()
            && channels.contains_key(tag)
        {
            return Some(tag.to_owned());
        }
        if channels.contains_key("latest") {
            return Some("latest".to_owned());
        }
        return channels.keys().next().cloned();
    }
    tool.dist_tag
        .clone()
        .or_else(|| Some("latest".to_owned()))
}

/// The user's persisted version-line selection for a tool, if any.
fn selected_channel(tool_id: &str) -> Option<String> {
    channel_selections_lock()
        .lock()
        .ok()
        .and_then(|guard| guard.get(tool_id).cloned())
}

/// Load persisted version-line selections into process-wide state. Called once
/// during setup, alongside the other app-config initializers.
pub fn initialize(app: &AppHandle) {
    let loaded = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| load_channel_selections(&dir.join(CHANNEL_CONFIG_FILE_NAME)))
        .unwrap_or_default();
    if let Ok(mut guard) = channel_selections_lock().lock() {
        *guard = loaded;
    }
}

fn load_channel_selections(path: &Path) -> HashMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Switch a tool's version line. The channel must be one the signed feed
/// currently offers for the tool (falling back to the tool's configured
/// dist-tag when the feed carries no channel data), so an invalid selection is
/// rejected instead of persisted. Saved with owner-only permissions like the
/// other app configs.
pub async fn set_channel(app: &AppHandle, tool_id: &str, channel: &str) -> Result<(), String> {
    let tool = tool_by_id(tool_id)?;
    let feed_entry = crate::feed::tool_entry(tool_id).await.ok().flatten();
    let allowed: Vec<String> = match feed_entry.as_ref().and_then(|entry| entry.channels.as_ref()) {
        Some(channels) => channels.keys().cloned().collect(),
        None => vec![tool.dist_tag.clone().unwrap_or_else(|| "latest".to_owned())],
    };
    if !allowed.iter().any(|candidate| candidate == channel) {
        return Err(format!("{} 不存在版本线 {channel}", tool.display_name));
    }
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法确定 UManager 配置目录：{error}"))?;
    std::fs::create_dir_all(&dir).map_err(|error| format!("无法创建 UManager 配置目录：{error}"))?;
    let path = dir.join(CHANNEL_CONFIG_FILE_NAME);
    {
        let mut selections = channel_selections_lock()
            .lock()
            .map_err(|_| "无法读取版本线选择状态".to_owned())?;
        selections.insert(tool_id.to_owned(), channel.to_owned());
        let json = serde_json::to_string_pretty(&*selections)
            .map_err(|error| format!("无法编码版本线选择：{error}"))?;
        crate::translation::write_private(&path, json.as_bytes())
            .map_err(|error| format!("无法保存版本线选择：{error}"))?;
    }
    Ok(())
}

fn find_binary(binary_name: &str, home: &Path) -> Option<PathBuf> {
    let mut seen = Vec::new();
    if let Some(found) = find_on_path(binary_name) {
        return Some(found);
    }
    for dir in known_binary_dirs(home) {
        let candidate = dir.join(binary_name);
        if candidate.is_file() && !seen.contains(&candidate) {
            seen.push(candidate.clone());
            return Some(candidate);
        }
    }
    None
}

/// Resolve an installed tool binary. Prefers the bin directory of the npm that
/// `resolve_npm` chooses — the nvm version currently in use — so detection
/// follows the active nvm install rather than a stale inherited `PATH`; then
/// falls back to `find_binary` (on-`PATH` + known install dirs) for non-nvm
/// layouts. The npm-resolved path also keeps npm-global installs under nvm
/// (whose bin directory is not in the GUI process `PATH`) visible, covering both
/// state detection and self-update resolution.
fn find_tool_binary(tool: &DevelopmentTool, home: &Path) -> Option<PathBuf> {
    if let Some(npm) = resolve_npm(home) {
        let candidate = npm.parent()?.join(&tool.binary_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    find_binary(&tool.binary_name, home)
}

fn classify_install_kind(
    tool: &DevelopmentTool,
    home: &Path,
    path: &Path,
    npm_available: bool,
) -> String {
    let official_dirs = [
        home.join(".local").join("bin").join(&tool.binary_name),
        home.join(".opencode").join("bin").join(&tool.binary_name),
        // pnpm 官方安装脚本布局，与其在 known_binary_dirs 中的登记保持一致。
        home.join(".local")
            .join("share")
            .join("pnpm")
            .join("bin")
            .join(&tool.binary_name),
    ];
    if official_dirs.iter().any(|candidate| candidate == path) {
        return "officialInstaller".to_owned();
    }
    if npm_available
        && tool
            .npm_package
            .as_deref()
            .is_some_and(|package| npm_has_package(home, package))
    {
        return "npmGlobal".to_owned();
    }
    "onPath".to_owned()
}

fn npm_has_package(home: &Path, package: &str) -> bool {
    npm_capture(
        home,
        &[
            "ls".to_owned(),
            "-g".to_owned(),
            package.to_owned(),
            "--depth=0".to_owned(),
        ],
    )
    .is_ok()
}

fn npm_installed_version(home: &Path, package: &str) -> Option<String> {
    npm_capture(
        home,
        &[
            "ls".to_owned(),
            "-g".to_owned(),
            package.to_owned(),
            "--json".to_owned(),
        ],
    )
    .ok()
    .and_then(|output| {
        let value: serde_json::Value = serde_json::from_str(&output).ok()?;
        let dependencies = value.get("dependencies")?;
        let entry = dependencies.get(package)?;
        entry.get("version")?.as_str().map(str::to_owned)
    })
}

fn capture_version(path: &Path) -> Option<String> {
    let output = Command::new(path)
        .arg("--version")
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    extract_version(&text).or_else(|| {
        if text.is_empty() {
            None
        } else {
            Some(text.lines().next().unwrap_or_default().trim().to_owned())
        }
    })
}

fn installer_label(tool: &DevelopmentTool) -> &'static str {
    match &tool.installer {
        DevToolInstaller::Npm => "npm 全局安装",
        DevToolInstaller::CurlScript { .. } => "官方安装脚本",
    }
}

/// Install an npm-distributed tool at an exact version when one is known (the
/// version the signed feed advertises for the selected version line — installs
/// and updates both land exactly on that version), falling back to the
/// configured dist-tag (`latest` by default; a tool may pin a pre-release
/// channel such as `next`) when the feed is unavailable.
fn install_command(
    tool: &DevelopmentTool,
    home: &Path,
    target_version: Option<&str>,
) -> Result<Command, String> {
    match &tool.installer {
        DevToolInstaller::Npm => {
            let package = tool
                .npm_package
                .as_deref()
                .ok_or_else(|| format!("{} 未配置 npm 包", tool.display_name))?;
            let spec = match target_version {
                Some(version) => format!("{package}@{version}"),
                None => {
                    let dist_tag = tool.dist_tag.as_deref().unwrap_or("latest");
                    format!("{package}@{dist_tag}")
                }
            };
            npm_command(home, &["install".to_owned(), "-g".to_owned(), spec])
        }
        DevToolInstaller::CurlScript {
            script_url, shell, ..
        } => {
            let shell = if shell == "sh" { "sh" } else { "bash" };
            let script = format!("curl -fsSL {} | {}", shell_quote(script_url), shell);
            let mut command = Command::new("/bin/bash");
            command
                .arg("-c")
                .arg(script)
                .stdin(Stdio::null())
                .env_clear()
                .env("PATH", SAFE_SYSTEM_PATH)
                .env("HOME", home)
                .env("LC_ALL", "C")
                .env("LANG", "C")
                .env("LANGUAGE", "C");
            apply_proxy_environment(&mut command);
            Ok(command)
        }
    }
}

fn update_command(
    tool: &DevelopmentTool,
    home: &Path,
    target_version: Option<&str>,
) -> Result<(Command, &'static str), String> {
    match &tool.update {
        Some(DevToolUpdate::SelfCommand { args }) => {
            Ok((binary_self_command(tool, home, args)?, "官方自更新命令"))
        }
        None => Ok((install_command(tool, home, target_version)?, installer_label(tool))),
    }
}

/// Run the already-installed tool binary with a fixed argument vector under a
/// clean environment (used for self-updates and the vendor's own
/// non-interactive uninstaller, e.g. `hermes uninstall --yes`).
fn binary_self_command(
    tool: &DevelopmentTool,
    home: &Path,
    args: &[String],
) -> Result<Command, String> {
    let binary = find_tool_binary(tool, home)
        .ok_or_else(|| format!("未检测到已安装的 {}", tool.display_name))?;
    let bin_dir = binary
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/usr/bin"));
    let mut command = Command::new(&binary);
    command
        .args(args)
        .env_clear()
        .env("PATH", format!("{}:{SAFE_SYSTEM_PATH}", bin_dir.display()))
        .env("HOME", home)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    apply_proxy_environment(&mut command);
    Ok(command)
}

fn uninstall_command(
    tool: &DevelopmentTool,
    home: &Path,
    install_kind: &str,
) -> Result<Command, String> {
    match install_kind {
        "npmGlobal" => npm_uninstall_command(tool, home),
        "officialInstaller" => match &tool.uninstall {
            DevToolUninstall::Npm => npm_uninstall_command(tool, home),
            DevToolUninstall::RemoveFiles { paths } => {
                let quoted = paths
                    .iter()
                    .map(|path| {
                        expand_path(path).map(|resolved| shell_quote(&resolved.to_string_lossy()))
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join(" ");
                let mut command = Command::new("/bin/bash");
                command
                    .arg("-c")
                    .arg(format!("rm -f {}", quoted))
                    .env_clear()
                    .env("PATH", SAFE_SYSTEM_PATH)
                    .env("HOME", home);
                apply_proxy_environment(&mut command);
                Ok(command)
            }
            DevToolUninstall::SelfCommand { args } => binary_self_command(tool, home, args),
        },
        other => Err(format!("无法卸载：安装来源（{other}）不在受支持的白名单内")),
    }
}

/// `npm uninstall -g <package>` for the tool's configured npm package.
fn npm_uninstall_command(tool: &DevelopmentTool, home: &Path) -> Result<Command, String> {
    let package = tool
        .npm_package
        .as_deref()
        .ok_or_else(|| format!("{} 未配置 npm 包", tool.display_name))?;
    npm_command(
        home,
        &[
            "uninstall".to_owned(),
            "-g".to_owned(),
            package.to_owned(),
        ],
    )
}

fn npm_available(home: &Path) -> bool {
    resolve_npm(home).is_some()
}

/// Resolve the npm executable. When nvm is installed, detection follows the nvm
/// version currently in use — `$NVM_BIN` (set by `nvm use`) first, then the
/// `default` alias resolved with nvm semantics (a partial prefix such as `24`
/// selects the highest installed version), then the newest install — so the
/// resolved npm matches the nvm version the user is actually using, rather than
/// a stale inherited `PATH`. Only when nvm isn't installed do we fall back to an
/// `npm` on `PATH` (system node / another manager).
fn resolve_npm(home: &Path) -> Option<PathBuf> {
    if let Some(nvm_dir) = nvm_dir(home) {
        let versions_dir = nvm_dir.join("versions").join("node");
        // The version currently active in this process environment.
        if let Some(nvm_bin) = std::env::var_os("NVM_BIN").map(PathBuf::from) {
            let npm = nvm_bin.join("npm");
            if npm.is_file() {
                return Some(npm);
            }
        }
        // The `default` alias, resolved like nvm (full version / partial prefix
        // / named LTS) — what a fresh shell would activate.
        if let Ok(alias) = std::fs::read_to_string(nvm_dir.join("alias").join("default")) {
            if let Some(version_dir) = resolve_nvm_default(&versions_dir, &nvm_dir, alias.trim()) {
                let npm = version_dir.join("bin").join("npm");
                if npm.is_file() {
                    return Some(npm);
                }
            }
        }
        // Newest installed version as a last resort within nvm.
        if let Some(newest) = newest_version_dir(&versions_dir) {
            let npm = newest.join("bin").join("npm");
            if npm.is_file() {
                return Some(npm);
            }
        }
        return None;
    }
    find_on_path("npm")
}

fn nvm_dir(home: &Path) -> Option<PathBuf> {
    std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .or_else(|| {
            let candidate = home.join(".nvm");
            candidate.is_dir().then_some(candidate)
        })
}

fn version_parts_from_npm_path(path: &Path) -> Vec<u64> {
    // The nvm version dir is a leaf like `v24.20.0`. A naive `starts_with('v')`
    // would also match the `versions` directory component in the path, so require
    // `v` followed by a digit and take the last such part.
    let path_text = path.to_string_lossy();
    let version = path_text
        .split('/')
        .filter(|part| {
            part.len() > 1 && part.starts_with('v') && part.as_bytes()[1].is_ascii_digit()
        })
        .last()
        .unwrap_or("v0");
    version_parts(version.trim_start_matches('v'))
}

/// Resolve an nvm `default` alias to a version directory, mirroring nvm: a full
/// version ("v24.20.0"), a partial prefix ("24"/"24.20" → highest match), or a
/// named LTS ("lts/*" / "lts/iron" → the mapped version).
fn resolve_nvm_default(versions_dir: &Path, nvm_dir: &Path, alias: &str) -> Option<PathBuf> {
    let alias = alias.trim();
    if let Some(name) = alias.strip_prefix("lts/") {
        let lts_dir = nvm_dir.join("alias").join("lts");
        if name == "*" {
            return highest_lts_version(&lts_dir, versions_dir);
        }
        let mapped = std::fs::read_to_string(lts_dir.join(name)).ok()?;
        return resolve_version_dir(versions_dir, mapped.trim());
    }
    resolve_version_dir(versions_dir, alias)
}

/// Match `version` (leading `v` optional) against the installed node versions:
/// an exact directory, or — for a partial prefix such as `24` — the highest
/// installed version sharing that component prefix.
fn resolve_version_dir(versions_dir: &Path, version: &str) -> Option<PathBuf> {
    let version = version.trim().trim_start_matches('v');
    if version.is_empty() {
        return None;
    }
    let exact = versions_dir.join(format!("v{version}"));
    if exact.is_dir() {
        return Some(exact);
    }
    let prefix = version_parts(version);
    let mut matches: Vec<PathBuf> = std::fs::read_dir(versions_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }
            let name = path.file_name()?.to_string_lossy();
            let candidate = name.strip_prefix('v')?;
            let parts = version_parts(candidate);
            if parts.len() < prefix.len() {
                return None;
            }
            (parts[..prefix.len()] == prefix[..]).then_some(path)
        })
        .collect();
    matches.sort_by_key(|path| version_parts_from_npm_path(path));
    matches.pop()
}

/// The highest installed node version directory under `versions_dir`.
fn newest_version_dir(versions_dir: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(versions_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.is_dir().then_some(path)
        })
        .collect();
    dirs.sort_by_key(|path| version_parts_from_npm_path(path));
    dirs.pop()
}

/// Highest version referenced by an nvm LTS alias file under `lts_dir` (each
/// file maps an LTS line name to a concrete version, e.g. `lts/iron` -> v24.20.0).
fn highest_lts_version(lts_dir: &Path, versions_dir: &Path) -> Option<PathBuf> {
    let mut mapped: Vec<PathBuf> = std::fs::read_dir(lts_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let version = std::fs::read_to_string(entry.path()).ok()?;
            resolve_version_dir(versions_dir, version.trim())
        })
        .collect();
    mapped.sort_by_key(|path| version_parts_from_npm_path(path));
    mapped.pop()
}

fn npm_command(home: &Path, args: &[String]) -> Result<Command, String> {
    let npm =
        resolve_npm(home).ok_or_else(|| "未检测到 npm，请先在“开发环境”安装 Node.js".to_owned())?;
    let bin_dir = npm
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/usr/bin"));
    let mut command = Command::new(&npm);
    command
        .args(args)
        .env_clear()
        .env("PATH", format!("{}:{SAFE_SYSTEM_PATH}", bin_dir.display()))
        .env("HOME", home)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    apply_proxy_environment(&mut command);
    Ok(command)
}

fn npm_capture(home: &Path, args: &[String]) -> Result<String, String> {
    let output = npm_command(home, args)?
        .output()
        .map_err(|error| format!("无法执行 npm：{error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn run_streaming(
    command: &mut Command,
    tool_id: &str,
    phase_message: &str,
    progress: Option<&DevToolProgressCallback>,
) -> Result<String, String> {
    if let Some(progress) = progress {
        progress(DevToolProgress {
            tool_id: tool_id.to_owned(),
            phase: "phase",
            stream: "system".to_owned(),
            message: phase_message.to_owned(),
        });
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动安装/卸载命令：{error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取命令输出".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取命令错误输出".to_owned())?;
    let collected = Arc::new(Mutex::new(Vec::<String>::new()));

    let stdout_thread = {
        let collected = Arc::clone(&collected);
        let progress = progress.cloned();
        let tool_id = tool_id.to_owned();
        std::thread::spawn(move || forward_lines(stdout, "stdout", &collected, progress, &tool_id))
    };
    let stderr_thread = {
        let collected = Arc::clone(&collected);
        let progress = progress.cloned();
        let tool_id = tool_id.to_owned();
        std::thread::spawn(move || forward_lines(stderr, "stderr", &collected, progress, &tool_id))
    };

    let status = child
        .wait()
        .map_err(|error| format!("无法等待命令结束：{error}"))?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let output = collected
        .lock()
        .map_err(|_| "无法读取命令输出".to_owned())?
        .join("\n");
    if let Some(progress) = progress {
        progress(DevToolProgress {
            tool_id: tool_id.to_owned(),
            phase: "completed",
            stream: "system".to_owned(),
            message: if status.success() {
                "操作已成功完成".to_owned()
            } else {
                "操作失败".to_owned()
            },
        });
    }
    if !status.success() {
        return Err(tail_summary(&output));
    }
    Ok(output)
}

fn forward_lines(
    reader: impl Read,
    stream: &'static str,
    collected: &Arc<Mutex<Vec<String>>>,
    progress: Option<DevToolProgressCallback>,
    tool_id: &str,
) {
    for line in BufReader::new(reader).split(b'\n') {
        let Ok(line) = line else { continue };
        let sanitized = sanitize_line(&String::from_utf8_lossy(&line));
        if sanitized.is_empty() {
            continue;
        }
        if let Ok(mut output) = collected.lock() {
            if output.len() < 200 {
                output.push(sanitized.clone());
            }
        }
        if let Some(progress) = &progress {
            progress(DevToolProgress {
                tool_id: tool_id.to_owned(),
                phase: "running",
                stream: stream.to_owned(),
                message: sanitized,
            });
        }
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn apply_proxy_environment(command: &mut Command) {
    for (key, value) in crate::network::proxy_environment() {
        command.env(key, value);
    }
}

fn user_home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "无法确定用户主目录".to_owned())
}

fn expand_path(value: &str) -> Result<PathBuf, String> {
    if value == "~" {
        return user_home();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(user_home()?.join(rest));
    }
    Ok(PathBuf::from(value))
}

fn extract_version(value: &str) -> Option<String> {
    // Find the first semver-ish token: a dotted numeric core, optionally followed
    // by a `-prerelease` suffix (e.g. `0.1.1-rc.2`). This keeps prerelease
    // versions intact so preview builds display their full version string.
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
            index += 1;
        }
        let numeric = &value[start..index];
        if !is_dotted_numeric(numeric) {
            continue;
        }
        // Optional prerelease: `-` followed by at least one [0-9A-Za-z.-] char.
        if index < bytes.len() && bytes[index] == b'-' {
            let prerelease_start = index;
            let mut end = index + 1;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'.' || bytes[end] == b'-')
            {
                end += 1;
            }
            if end > prerelease_start + 1 {
                index = end;
            }
        }
        return Some(value[start..index].to_owned());
    }
    None
}

fn is_dotted_numeric(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let (left_core, left_prerelease) = split_version(left);
    let (right_core, right_prerelease) = split_version(right);
    let left = version_parts(left_core);
    let right = version_parts(right_core);
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        match a.cmp(&b) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    compare_prerelease(left_prerelease, right_prerelease)
}

/// Split a version into its numeric core and an optional `-prerelease` suffix.
/// A trailing or empty prerelease is treated as absent.
fn split_version(value: &str) -> (&str, Option<&str>) {
    match value.split_once('-') {
        Some((core, prerelease)) if !prerelease.is_empty() => (core, Some(prerelease)),
        _ => (value, None),
    }
}

/// Semver prerelease precedence: a release without a prerelease outranks one
/// with a prerelease; otherwise compare dot-separated identifiers (numeric
/// identifiers compare numerically and rank below alphanumeric ones).
fn compare_prerelease(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => {
            let left_parts: Vec<&str> = left.split('.').collect();
            let right_parts: Vec<&str> = right.split('.').collect();
            for index in 0..left_parts.len().max(right_parts.len()) {
                match (left_parts.get(index), right_parts.get(index)) {
                    (Some(a), Some(b)) => {
                        let ordering = compare_prerelease_identifier(a, b);
                        if ordering != Ordering::Equal {
                            return ordering;
                        }
                    }
                    (Some(_), None) => return Ordering::Greater,
                    (None, Some(_)) => return Ordering::Less,
                    (None, None) => unreachable!(),
                }
            }
            Ordering::Equal
        }
    }
}

fn compare_prerelease_identifier(left: &str, right: &str) -> Ordering {
    let left_numeric = left.bytes().all(|byte| byte.is_ascii_digit());
    let right_numeric = right.bytes().all(|byte| byte.is_ascii_digit());
    match (left_numeric, right_numeric) {
        (true, true) => left
            .parse::<u64>()
            .unwrap_or(0)
            .cmp(&right.parse::<u64>().unwrap_or(0)),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => left.cmp(right),
    }
}

fn version_parts(value: &str) -> Vec<u64> {
    value
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn sanitize_line(input: &str) -> String {
    let mut output = String::with_capacity(input.len().min(MAX_LOG_LINE_CHARS));
    for character in input.chars().take(MAX_LOG_LINE_CHARS) {
        if character == '\t' {
            output.push_str("    ");
        } else if !character.is_control() {
            output.push(character);
        }
    }
    output
}

fn tail_summary(output: &str) -> String {
    let lines = output.lines().collect::<Vec<_>>();
    lines
        .iter()
        .rev()
        .take(3)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_numeric_versions() {
        assert_eq!(
            extract_version("2.1.245 (Claude Code)"),
            Some("2.1.245".to_owned())
        );
        assert_eq!(
            extract_version("opencode 1.18.22"),
            Some("1.18.22".to_owned())
        );
        assert_eq!(extract_version("0.149.1"), Some("0.149.1".to_owned()));
        assert_eq!(extract_version("v24.19.0"), Some("24.19.0".to_owned()));
        assert_eq!(extract_version("no version here"), None);
    }

    #[test]
    fn extracts_prerelease_versions_in_full() {
        assert_eq!(
            extract_version("0.1.1-rc.2"),
            Some("0.1.1-rc.2".to_owned())
        );
        assert_eq!(
            extract_version("0.1.2-alpha.1"),
            Some("0.1.2-alpha.1".to_owned())
        );
        assert_eq!(
            extract_version("dsh 0.1.1-rc.2 (DeepSeek Harness)"),
            Some("0.1.1-rc.2".to_owned())
        );
    }

    #[test]
    fn compares_versions_component_wise() {
        assert_eq!(compare_versions("1.18.22", "1.18.22"), Ordering::Equal);
        assert_eq!(compare_versions("1.18.22", "1.19.0"), Ordering::Less);
        assert_eq!(compare_versions("2.1.245", "2.1.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.84.3", "0.84.10"), Ordering::Less);
    }

    #[test]
    fn compares_prerelease_versions_with_semver_precedence() {
        assert_eq!(compare_versions("0.1.1-rc.2", "0.1.1-rc.2"), Ordering::Equal);
        assert_eq!(compare_versions("0.1.1-rc.2", "0.1.1-rc.3"), Ordering::Less);
        assert_eq!(compare_versions("0.1.1-rc.2", "0.1.1"), Ordering::Less);
        assert_eq!(compare_versions("0.1.1", "0.1.1-rc.2"), Ordering::Greater);
        assert_eq!(compare_versions("0.1.1-rc.2", "0.1.2-rc.1"), Ordering::Less);
        assert_eq!(compare_versions("0.1.1-alpha.1", "0.1.1-rc.1"), Ordering::Less);
    }

    #[test]
    fn embedded_tools_are_configured() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.development_tools.len(), 8);
        assert!(catalog.by_tool_id("claude-code").is_some());
        assert!(catalog.by_tool_id("opencode").is_some());
        assert!(catalog.by_tool_id("pi").is_some());
        assert!(catalog.by_tool_id("codex").is_some());
        assert!(catalog.by_tool_id("dsh").is_some());
        assert!(catalog.by_tool_id("hermes").is_some());
        assert!(catalog.by_tool_id("uv").is_some());
        assert!(catalog.by_tool_id("pnpm").is_some());
    }

    fn dsh_tool(dist_tag: Option<&str>) -> DevelopmentTool {
        DevelopmentTool {
            tool_id: "dsh".to_owned(),
            display_name: "DeepSeek Harness".to_owned(),
            vendor: "DeepSeek".to_owned(),
            description: None,
            homepage: "https://github.com/deepseek-ai/deepseek-harness".to_owned(),
            icon: None,
            accent_color: None,
            binary_name: "dsh".to_owned(),
            npm_package: Some("@deepseek-ai/dsh".to_owned()),
            dist_tag: dist_tag.map(str::to_owned),
            installer: DevToolInstaller::Npm,
            uninstall: DevToolUninstall::Npm,
            update: None,
        }
    }

    fn dsh_feed_entry(channels: Option<&[(&str, &str)]>) -> crate::feed::FeedToolEntry {
        crate::feed::FeedToolEntry {
            npm_package: Some("@deepseek-ai/dsh".to_owned()),
            version: "0.1.2-rc.1".to_owned(),
            channels: channels.map(|list| {
                list.iter()
                    .map(|(tag, version)| (tag.to_string(), version.to_string()))
                    .collect()
            }),
            channel_release_notes: None,
            version_updated_at_unix_seconds: None,
            version_updated_at_source: None,
            release_notes: None,
            release_notes_url: None,
        }
    }

    fn dsh_entry_with_channel_notes() -> crate::feed::FeedToolEntry {
        let mut entry = dsh_feed_entry(Some(&[("latest", "0.1.2-rc.1"), ("alpha", "0.1.3-alpha.2")]));
        entry.release_notes = Some("rc 默认线更新记录".to_owned());
        entry.release_notes_url = Some("https://github.com/deepseek-ai/deepseek-harness/releases/tag/dsh-v0.1.2-rc.1".to_owned());
        let mut channel_notes = std::collections::BTreeMap::new();
        channel_notes.insert(
            "0.1.3-alpha.2".to_owned(),
            crate::feed::FeedChannelReleaseNotes {
                release_notes: Some("alpha 线更新记录".to_owned()),
                release_notes_url: Some("https://github.com/deepseek-ai/deepseek-harness/releases/tag/dsh-v0.1.3-alpha.2".to_owned()),
            },
        );
        entry.channel_release_notes = Some(channel_notes);
        entry
    }

    #[test]
    fn effective_channel_prefers_persisted_selection_when_offered() {
        let entry = dsh_feed_entry(Some(&[("latest", "0.1.2-rc.1"), ("alpha", "0.1.3-alpha.2")]));
        let channels = entry.channels.as_ref();
        // Persisted selection that the feed still offers wins.
        assert_eq!(
            effective_channel(&dsh_tool(Some("latest")), channels, Some("alpha")),
            Some("alpha".to_owned())
        );
        // A stale selection the feed no longer offers falls back to distTag.
        assert_eq!(
            effective_channel(&dsh_tool(Some("latest")), channels, Some("beta")),
            Some("latest".to_owned())
        );
        // No persisted selection -> configured distTag.
        assert_eq!(
            effective_channel(&dsh_tool(Some("alpha")), channels, None),
            Some("alpha".to_owned())
        );
        // No distTag -> `latest`.
        assert_eq!(
            effective_channel(&dsh_tool(None), channels, None),
            Some("latest".to_owned())
        );
        // No channels at all (older feed / non-npm tool) -> distTag.
        assert_eq!(
            effective_channel(&dsh_tool(Some("latest")), None, Some("alpha")),
            Some("latest".to_owned())
        );
        // Configured distTag not offered by the feed -> conventional `latest`.
        assert_eq!(
            effective_channel(&dsh_tool(Some("beta")), channels, None),
            Some("latest".to_owned())
        );
        // No distTag, no `latest` line -> the first offered channel.
        let without_latest = dsh_feed_entry(Some(&[("alpha", "0.1.3-alpha.2"), ("next", "0.1.2-rc.1")]));
        assert_eq!(
            effective_channel(&dsh_tool(None), without_latest.channels.as_ref(), None),
            Some("alpha".to_owned())
        );
    }

    #[test]
    fn target_version_follows_the_selected_channel() {
        let entry = dsh_feed_entry(Some(&[("latest", "0.1.2-rc.1"), ("alpha", "0.1.3-alpha.2")]));
        // Selected channel's exact version.
        assert_eq!(
            target_version_for(Some(&entry), Some("alpha")),
            Some("0.1.3-alpha.2".to_owned())
        );
        // No selection -> the feed's resolved default version.
        assert_eq!(
            target_version_for(Some(&entry), None),
            Some("0.1.2-rc.1".to_owned())
        );
        // Selection the feed does not offer -> default version.
        assert_eq!(
            target_version_for(Some(&entry), Some("beta")),
            Some("0.1.2-rc.1".to_owned())
        );
        // Feed entry without channels -> its version.
        let plain = dsh_feed_entry(None);
        assert_eq!(
            target_version_for(Some(&plain), Some("alpha")),
            Some("0.1.2-rc.1".to_owned())
        );
        // No feed entry at all (feed unavailable) -> None, caller falls back to distTag.
        assert_eq!(target_version_for(None, Some("alpha")), None);
    }

    #[test]
    fn release_notes_follow_the_selected_version_line() {
        let entry = dsh_entry_with_channel_notes();
        // Selected channel version has per-channel notes -> those win.
        let (notes, url) = release_notes_for(Some(&entry), Some("0.1.3-alpha.2"));
        assert_eq!(notes.as_deref(), Some("alpha 线更新记录"));
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/deepseek-ai/deepseek-harness/releases/tag/dsh-v0.1.3-alpha.2")
        );
        // Selected version without channel notes (or the default line) ->
        // entry-level notes.
        let (notes, _) = release_notes_for(Some(&entry), Some("0.1.2-rc.1"));
        assert_eq!(notes.as_deref(), Some("rc 默认线更新记录"));
        let (notes, _) = release_notes_for(Some(&entry), None);
        assert_eq!(notes.as_deref(), Some("rc 默认线更新记录"));
        // No feed entry -> no notes.
        let (notes, url) = release_notes_for(None, Some("0.1.3-alpha.2"));
        assert!(notes.is_none());
        assert!(url.is_none());
    }

    #[test]
    fn resolves_exact_and_partial_nvm_default_aliases() {
        let root = std::env::temp_dir().join(format!("umanager-nvm-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let versions = root.join("versions").join("node");
        for v in ["v22.14.0", "v24.19.0", "v24.20.0"] {
            std::fs::create_dir_all(versions.join(v)).unwrap();
        }
        let dir_name = |p: Option<PathBuf>| {
            p.map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        };
        // Exact full version (with or without the leading `v`).
        assert_eq!(
            dir_name(resolve_version_dir(&versions, "v24.19.0")),
            Some("v24.19.0".to_owned())
        );
        assert_eq!(
            dir_name(resolve_version_dir(&versions, "24.20.0")),
            Some("v24.20.0".to_owned())
        );
        // Partial prefix -> highest matching version, not the lowest.
        assert_eq!(dir_name(resolve_version_dir(&versions, "24")), Some("v24.20.0".to_owned()));
        assert_eq!(dir_name(resolve_version_dir(&versions, "v24")), Some("v24.20.0".to_owned()));
        assert_eq!(dir_name(resolve_version_dir(&versions, "24.19")), Some("v24.19.0".to_owned()));
        // Empty / no match.
        assert_eq!(dir_name(resolve_version_dir(&versions, "")), None);
        assert_eq!(dir_name(resolve_version_dir(&versions, "18")), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolves_named_lts_default_alias() {
        let root = std::env::temp_dir().join(format!("umanager-nvm-lts-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let versions = root.join("versions").join("node");
        std::fs::create_dir_all(versions.join("v22.14.0")).unwrap();
        std::fs::create_dir_all(versions.join("v24.20.0")).unwrap();
        let lts_dir = root.join("alias").join("lts");
        std::fs::create_dir_all(&lts_dir).unwrap();
        std::fs::write(lts_dir.join("iron"), "v24.20.0\n").unwrap();

        let dir_name = |p: Option<PathBuf>| {
            p.map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        };
        // lts/* -> highest LTS-mapped version.
        assert_eq!(
            dir_name(highest_lts_version(&lts_dir, &versions)),
            Some("v24.20.0".to_owned())
        );
        // lts/<name> -> the mapped version.
        assert_eq!(
            dir_name(resolve_nvm_default(&versions, &root, "lts/iron")),
            Some("v24.20.0".to_owned())
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
