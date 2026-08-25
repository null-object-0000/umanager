use serde::Serialize;
use std::cmp::Ordering;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use umanager_catalog::{Catalog, DevToolInstaller, DevToolUninstall, DevelopmentTool};

const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
const MAX_LOG_LINE_CHARS: usize = 2_000;

/// Where the current user keeps officially-installed CLI binaries. These are the
/// locations used by the vendor installers configured in `vendors.json`.
fn known_binary_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local").join("bin"),
        home.join(".opencode").join("bin"),
        home.join(".npm-global").join("bin"),
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
    pub npm_package: String,
    /// `npm` or `curlScript`, mirroring the configured installer.
    pub installer_kind: String,
    pub npm_available: bool,
    pub installed: bool,
    /// `npmGlobal`, `officialInstaller`, `onPath` or `null` when not installed.
    pub install_kind: Option<String>,
    pub version: Option<String>,
    pub latest_version: Option<String>,
    pub binary_path: Option<String>,
    pub update_available: bool,
    pub can_uninstall: bool,
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
    let lookup_id = tool.tool_id.clone();
    let lookup_package = tool.npm_package.clone();
    let feed_version = crate::feed::tool_entry(&lookup_id)
        .await
        .ok()
        .flatten()
        .filter(|entry| entry.npm_package == lookup_package)
        .map(|entry| entry.version);
    tauri::async_runtime::spawn_blocking(move || detect_state_sync(&tool, feed_version))
        .await
        .map_err(|error| format!("命令行工具检测任务异常结束：{error}"))?
}

pub async fn install(
    tool_id: String,
    progress: DevToolProgressCallback,
) -> Result<DevToolReport, String> {
    let tool = tool_by_id(&tool_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let home = user_home()?;
        let mut command = install_command(&tool, &home)?;
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

fn detect_state_sync(tool: &DevelopmentTool, feed_version: Option<String>) -> Result<DevToolState, String> {
    let home = user_home()?;
    let npm_available = npm_available(&home);
    let latest_version = feed_version.or_else(|| {
        if npm_available {
            npm_capture(
                &home,
                &[
                    "view".to_owned(),
                    tool.npm_package.clone(),
                    "version".to_owned(),
                ],
            )
            .ok()
            .map(|value| extract_version(&value).unwrap_or_else(|| value))
        } else {
            None
        }
    });

    let binary = find_binary(&tool.binary_name, &home);
    let install_kind = binary
        .as_ref()
        .map(|path| classify_install_kind(tool, &home, path, npm_available));
    let version = binary
        .as_ref()
        .and_then(|path| capture_version(path))
        .or_else(|| {
            if install_kind.as_deref() == Some("npmGlobal") && npm_available {
                npm_installed_version(&home, &tool.npm_package)
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
        binary_path: binary.map(|path| path.to_string_lossy().into_owned()),
        update_available,
        can_uninstall,
    })
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

fn classify_install_kind(
    tool: &DevelopmentTool,
    home: &Path,
    path: &Path,
    npm_available: bool,
) -> String {
    let official_dirs = [
        home.join(".local").join("bin").join(&tool.binary_name),
        home.join(".opencode").join("bin").join(&tool.binary_name),
    ];
    if official_dirs.iter().any(|candidate| candidate == path) {
        return "officialInstaller".to_owned();
    }
    if npm_available && npm_has_package(home, &tool.npm_package) {
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

fn installer_label(tool: &DevelopmentTool) -> &str {
    match &tool.installer {
        DevToolInstaller::Npm => "npm 全局安装",
        DevToolInstaller::CurlScript { .. } => "官方安装脚本",
    }
}

fn install_command(tool: &DevelopmentTool, home: &Path) -> Result<Command, String> {
    match &tool.installer {
        DevToolInstaller::Npm => npm_command(
            home,
            &[
                "install".to_owned(),
                "-g".to_owned(),
                format!("{}@latest", tool.npm_package),
            ],
        ),
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

fn uninstall_command(
    tool: &DevelopmentTool,
    home: &Path,
    install_kind: &str,
) -> Result<Command, String> {
    match install_kind {
        "npmGlobal" => npm_command(
            home,
            &[
                "uninstall".to_owned(),
                "-g".to_owned(),
                tool.npm_package.clone(),
            ],
        ),
        "officialInstaller" => match &tool.uninstall {
            DevToolUninstall::Npm => npm_command(
                home,
                &[
                    "uninstall".to_owned(),
                    "-g".to_owned(),
                    tool.npm_package.clone(),
                ],
            ),
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
        },
        other => Err(format!("无法卸载：安装来源（{other}）不在受支持的白名单内")),
    }
}

fn npm_available(home: &Path) -> bool {
    resolve_npm(home).is_some()
}

/// Resolve the npm executable: prefer one already on `PATH`, then fall back to the
/// nvm-installed Node.js that UManager manages (default alias first, then newest).
fn resolve_npm(home: &Path) -> Option<PathBuf> {
    if let Some(npm) = find_on_path("npm") {
        return Some(npm);
    }
    let nvm_dir = nvm_dir(home)?;
    let versions_dir = nvm_dir.join("versions").join("node");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&versions_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let npm = entry.path().join("bin").join("npm");
            npm.is_file().then_some(npm)
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }

    if let Ok(alias) = std::fs::read_to_string(nvm_dir.join("alias").join("default")) {
        let version = alias.trim().trim_start_matches('v');
        let preferred = versions_dir
            .join(format!("v{version}"))
            .join("bin")
            .join("npm");
        if preferred.is_file() {
            return Some(preferred);
        }
    }

    candidates.sort_by_key(|path| version_parts_from_npm_path(path));
    candidates.pop()
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
    let path_text = path.to_string_lossy();
    let version = path_text
        .split('/')
        .find(|part| part.starts_with('v') && part.len() > 1)
        .unwrap_or("v0");
    version_parts(version.trim_start_matches('v'))
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
    for token in value.split(|character: char| !(character.is_ascii_digit() || character == '.')) {
        let token = token.trim_matches('.');
        if token.is_empty() {
            continue;
        }
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() >= 2
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return Some(token.to_owned());
        }
    }
    None
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = version_parts(left);
    let right = version_parts(right);
    for index in 0..left.len().max(right.len()) {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        match a.cmp(&b) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
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
    fn compares_versions_component_wise() {
        assert_eq!(compare_versions("1.18.22", "1.18.22"), Ordering::Equal);
        assert_eq!(compare_versions("1.18.22", "1.19.0"), Ordering::Less);
        assert_eq!(compare_versions("2.1.245", "2.1.9"), Ordering::Greater);
        assert_eq!(compare_versions("0.84.3", "0.84.10"), Ordering::Less);
    }

    #[test]
    fn embedded_tools_are_configured() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.development_tools.len(), 4);
        assert!(catalog.by_tool_id("claude-code").is_some());
        assert!(catalog.by_tool_id("opencode").is_some());
        assert!(catalog.by_tool_id("pi").is_some());
        assert!(catalog.by_tool_id("codex").is_some());
    }
}
