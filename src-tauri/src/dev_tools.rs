use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use umanager_catalog::{Catalog, DevelopmentToolchain, ManagerKind};

const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
const MAX_LOG_LINE_CHARS: usize = 2_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevToolchainState {
    pub toolchain_id: String,
    pub display_name: String,
    pub vendor: String,
    pub homepage: String,
    pub manager: String,
    pub manager_found: bool,
    pub manager_home: Option<String>,
    pub manager_version: Option<String>,
    pub default_version: Option<String>,
    pub installed_versions: Vec<DevVersion>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevVersion {
    pub version: String,
    pub is_default: bool,
    pub is_lts: bool,
    pub lts_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevRelease {
    pub version: String,
    pub label: Option<String>,
    pub recommended: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevOperationReport {
    pub toolchain_id: String,
    pub action: String,
    pub version: String,
    pub success: bool,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevOperationProgress {
    pub toolchain_id: String,
    pub phase: &'static str,
    pub stream: String,
    pub message: String,
}

pub type DevProgressCallback = Arc<dyn Fn(DevOperationProgress) + Send + Sync>;

pub fn load_toolchains() -> Result<Vec<DevelopmentToolchain>, String> {
    Ok(Catalog::load()?.development_toolchains)
}

pub async fn detect_state(toolchain_id: String) -> Result<DevToolchainState, String> {
    let catalog = Catalog::load()?;
    let toolchain = catalog
        .by_toolchain_id(&toolchain_id)
        .cloned()
        .ok_or_else(|| format!("软件源中不存在开发工具 {toolchain_id}"))?;
    tauri::async_runtime::spawn_blocking(move || detect_state_sync(&toolchain))
        .await
        .map_err(|error| format!("开发工具检测任务异常结束：{error}"))?
}

pub async fn list_remote_versions(toolchain_id: String) -> Result<Vec<DevRelease>, String> {
    let (toolchain, home, data_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || list_remote_versions_sync(&toolchain, &home, &data_dir))
        .await
        .map_err(|error| format!("远程版本读取任务异常结束：{error}"))?
}

pub async fn install_version(
    toolchain_id: String,
    version: String,
    progress: DevProgressCallback,
) -> Result<DevOperationReport, String> {
    validate_version_token(&version)?;
    let (toolchain, home, data_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let output = run_sequence(&toolchain, &home, &data_dir, &install_commands(&toolchain, &version), Some(&progress))?;
        Ok(DevOperationReport {
            toolchain_id: toolchain.toolchain_id.clone(),
            action: "install".to_owned(),
            version,
            success: true,
            message: tail_summary(&output),
        })
    })
    .await
    .map_err(|error| format!("版本安装任务异常结束：{error}"))?
}

pub async fn set_default_version(
    toolchain_id: String,
    version: String,
    progress: DevProgressCallback,
) -> Result<DevOperationReport, String> {
    validate_version_token(&version)?;
    let (toolchain, home, data_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let output = run_sequence(&toolchain, &home, &data_dir, &set_default_commands(&toolchain, &version), Some(&progress))?;
        Ok(DevOperationReport {
            toolchain_id: toolchain.toolchain_id.clone(),
            action: "set-default".to_owned(),
            version,
            success: true,
            message: tail_summary(&output),
        })
    })
    .await
    .map_err(|error| format!("切换默认版本任务异常结束：{error}"))?
}

pub async fn uninstall_version(
    toolchain_id: String,
    version: String,
    progress: DevProgressCallback,
) -> Result<DevOperationReport, String> {
    validate_version_token(&version)?;
    let (toolchain, home, data_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let output = run_sequence(&toolchain, &home, &data_dir, &uninstall_commands(&toolchain, &version), Some(&progress))?;
        Ok(DevOperationReport {
            toolchain_id: toolchain.toolchain_id.clone(),
            action: "uninstall".to_owned(),
            version,
            success: true,
            message: tail_summary(&output),
        })
    })
    .await
    .map_err(|error| format!("版本卸载任务异常结束：{error}"))?
}

fn prepare(toolchain_id: &str) -> Result<(DevelopmentToolchain, PathBuf, PathBuf), String> {
    let catalog = Catalog::load()?;
    let toolchain = catalog
        .by_toolchain_id(toolchain_id)
        .cloned()
        .ok_or_else(|| format!("软件源中不存在开发工具 {toolchain_id}"))?;
    let home = user_home()?;
    let data_dir = resolve_data_dir(&toolchain)
        .ok_or_else(|| format!("未检测到 {}（{}）", toolchain.display_name, toolchain.manager_home))?;
    Ok((toolchain, home, data_dir))
}

fn detect_state_sync(toolchain: &DevelopmentToolchain) -> Result<DevToolchainState, String> {
    let home = user_home()?;
    let data_dir = resolve_data_dir(toolchain);
    let manager_found = data_dir.as_ref().is_some_and(|dir| manager_available(toolchain, dir));
    let manager_version = if manager_found {
        manager_capture(toolchain, &home, data_dir.as_ref().unwrap(), &["--version".to_owned()])
            .ok()
            .map(|value| parse_manager_version(toolchain, &value))
    } else {
        None
    };
    let installed = data_dir
        .as_ref()
        .map(|dir| list_installed_versions(toolchain, dir))
        .unwrap_or_default();
    let lts_aliases = if toolchain.manager_kind == ManagerKind::Shell {
        data_dir.as_ref().map(|dir| lts_aliases(dir.as_path())).unwrap_or_default()
    } else {
        HashMap::new()
    };
    let default_version = if manager_found {
        resolve_default_version(toolchain, &home, data_dir.as_ref().unwrap())
    } else {
        None
    };

    let mut installed_versions = installed
        .into_iter()
        .map(|version| DevVersion {
            is_default: default_version.as_deref() == Some(version.as_str()),
            is_lts: lts_aliases.contains_key(&version),
            lts_name: lts_aliases.get(&version).cloned(),
            version,
        })
        .collect::<Vec<_>>();
    if toolchain.manager_kind == ManagerKind::Shell {
        installed_versions.sort_by(|left, right| version_key(&right.version).cmp(&version_key(&left.version)));
    } else {
        installed_versions.sort_by(|left, right| channel_key(&left.version).cmp(&channel_key(&right.version)));
    }

    Ok(DevToolchainState {
        toolchain_id: toolchain.toolchain_id.clone(),
        display_name: toolchain.display_name.clone(),
        vendor: toolchain.vendor.clone(),
        homepage: toolchain.homepage.clone(),
        manager: toolchain.manager.clone(),
        manager_found,
        manager_home: data_dir.as_ref().map(|dir| dir.to_string_lossy().into_owned()),
        manager_version,
        default_version,
        installed_versions,
    })
}

fn manager_available(toolchain: &DevelopmentToolchain, data_dir: &Path) -> bool {
    match toolchain.manager_kind {
        ManagerKind::Shell => toolchain
            .manager_script
            .as_ref()
            .is_some_and(|script| data_dir.join(script).is_file()),
        ManagerKind::Binary => resolve_binary(toolchain).is_some(),
    }
}

fn list_installed_versions(toolchain: &DevelopmentToolchain, data_dir: &Path) -> Vec<String> {
    let directory = expand_path(&toolchain.versions_directory)
        .unwrap_or_else(|_| data_dir.join("versions").join("node"));
    let Ok(entries) = fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut versions = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.path().is_dir() || name.is_empty() {
                return None;
            }
            match toolchain.manager_kind {
                ManagerKind::Shell => (name.starts_with('v') && name.len() > 1).then_some(name),
                ManagerKind::Binary => Some(shorten_toolchain(&name)),
            }
        })
        .collect::<Vec<_>>();
    versions.dedup();
    versions
}

fn lts_aliases(nvm_dir: &Path) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    let lts_dir = nvm_dir.join("alias").join("lts");
    let Ok(entries) = fs::read_dir(&lts_dir) else {
        return aliases;
    };
    for entry in entries.flatten() {
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let version = content.trim().to_owned();
        if is_concrete_version(&version) {
            let codename = entry.file_name().to_string_lossy().into_owned();
            aliases.insert(version, codename);
        }
    }
    aliases
}

fn list_remote_versions_sync(
    toolchain: &DevelopmentToolchain,
    home: &Path,
    data_dir: &Path,
) -> Result<Vec<DevRelease>, String> {
    match toolchain.manager_kind {
        ManagerKind::Shell => {
            let output = manager_capture(
                toolchain,
                home,
                data_dir,
                &["ls-remote".to_owned(), "--lts".to_owned()],
            )?;
            Ok(parse_lts_releases(&output))
        }
        ManagerKind::Binary => Ok(rust_channels()),
    }
}

fn rust_channels() -> Vec<DevRelease> {
    vec![
        DevRelease { version: "stable".to_owned(), label: Some("稳定版".to_owned()), recommended: true },
        DevRelease { version: "beta".to_owned(), label: Some("测试版".to_owned()), recommended: false },
        DevRelease { version: "nightly".to_owned(), label: Some("每日版".to_owned()), recommended: false },
    ]
}

fn parse_lts_releases(output: &str) -> Vec<DevRelease> {
    let mut by_major: HashMap<u32, (String, String, bool)> = HashMap::new();
    for line in output.lines() {
        if let Some((version, major, codename, _latest)) = parse_lts_line(line) {
            by_major.insert(major, (version, codename, false));
        }
    }
    let mut releases = by_major
        .into_iter()
        .map(|(_, (version, codename, _))| DevRelease {
            version,
            label: Some(format!("LTS {codename}")),
            recommended: false,
        })
        .collect::<Vec<_>>();
    if let Some(latest) = releases
        .iter()
        .max_by(|left, right| version_key(&left.version).cmp(&version_key(&right.version)))
        .map(|release| release.version.clone())
    {
        for release in &mut releases {
            release.recommended = release.version == latest;
        }
    }
    releases.sort_by(|left, right| version_key(&right.version).cmp(&version_key(&left.version)));
    releases
}

fn parse_lts_line(line: &str) -> Option<(String, u32, String, bool)> {
    let version = line
        .split_whitespace()
        .find(|word| word.strip_prefix('v').is_some_and(is_version))?
        .to_owned();
    let major = version.trim_start_matches('v').split('.').next()?.parse().ok()?;
    let codename = line
        .split("LTS:")
        .nth(1)?
        .trim()
        .trim_end_matches(')')
        .to_owned();
    if codename.is_empty() {
        return None;
    }
    Some((version, major, codename, line.contains("Latest LTS")))
}

fn resolve_default_version(toolchain: &DevelopmentToolchain, home: &Path, data_dir: &Path) -> Option<String> {
    match toolchain.manager_kind {
        ManagerKind::Shell => manager_capture(toolchain, home, data_dir, &["version".to_owned(), "default".to_owned()])
            .ok()
            .filter(|value| !value.is_empty()),
        ManagerKind::Binary => manager_capture(toolchain, home, data_dir, &["default".to_owned()])
            .ok()
            .and_then(|value| value.split_whitespace().next().map(str::to_owned))
            .map(|name| shorten_toolchain(&name)),
    }
}

fn install_commands(toolchain: &DevelopmentToolchain, version: &str) -> Vec<Vec<String>> {
    match toolchain.manager_kind {
        ManagerKind::Shell => vec![vec!["install".to_owned(), version.to_owned(), "--default".to_owned()]],
        ManagerKind::Binary => vec![
            vec!["toolchain".to_owned(), "install".to_owned(), version.to_owned()],
            vec!["default".to_owned(), version.to_owned()],
        ],
    }
}

fn set_default_commands(toolchain: &DevelopmentToolchain, version: &str) -> Vec<Vec<String>> {
    match toolchain.manager_kind {
        ManagerKind::Shell => vec![vec!["alias".to_owned(), "default".to_owned(), version.to_owned()]],
        ManagerKind::Binary => vec![vec!["default".to_owned(), version.to_owned()]],
    }
}

fn uninstall_commands(toolchain: &DevelopmentToolchain, version: &str) -> Vec<Vec<String>> {
    match toolchain.manager_kind {
        ManagerKind::Shell => vec![vec!["uninstall".to_owned(), version.to_owned()]],
        ManagerKind::Binary => vec![vec!["toolchain".to_owned(), "uninstall".to_owned(), version.to_owned()]],
    }
}

fn run_sequence(
    toolchain: &DevelopmentToolchain,
    home: &Path,
    data_dir: &Path,
    commands: &[Vec<String>],
    progress: Option<&DevProgressCallback>,
) -> Result<String, String> {
    let mut combined = Vec::new();
    for command in commands {
        let output = run_manager_streaming(toolchain, home, data_dir, command, progress)?;
        combined.push(output);
    }
    Ok(combined.join("\n"))
}

fn run_manager_streaming(
    toolchain: &DevelopmentToolchain,
    home: &Path,
    data_dir: &Path,
    args: &[String],
    progress: Option<&DevProgressCallback>,
) -> Result<String, String> {
    let mut command = manager_command(toolchain, home, data_dir, args)?;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动 {}：{error}", toolchain.display_name))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("无法读取 {} 输出", toolchain.display_name))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("无法读取 {} 错误输出", toolchain.display_name))?;
    let collected = Arc::new(Mutex::new(Vec::<String>::new()));
    let toolchain_id = toolchain.toolchain_id.clone();

    let stdout_thread = {
        let collected = Arc::clone(&collected);
        let progress = progress.cloned();
        let toolchain_id = toolchain_id.clone();
        std::thread::spawn(move || forward_lines(stdout, "stdout", &collected, progress, &toolchain_id))
    };
    let stderr_thread = {
        let collected = Arc::clone(&collected);
        let progress = progress.cloned();
        let toolchain_id = toolchain_id.clone();
        std::thread::spawn(move || forward_lines(stderr, "stderr", &collected, progress, &toolchain_id))
    };

    let status = child
        .wait()
        .map_err(|error| format!("无法等待 {}：{error}", toolchain.display_name))?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let output = collected
        .lock()
        .map_err(|_| "无法读取开发工具输出".to_owned())?
        .join("\n");
    if let Some(progress) = progress {
        progress(DevOperationProgress {
            toolchain_id,
            phase: "completed",
            stream: "system".to_owned(),
            message: if status.success() { "操作已成功完成".to_owned() } else { "操作失败".to_owned() },
        });
    }
    if !status.success() {
        return Err(format!("{} 操作失败：{}", toolchain.display_name, tail_summary(&output)));
    }
    Ok(output)
}

fn forward_lines(
    reader: impl Read,
    stream: &'static str,
    collected: &Arc<Mutex<Vec<String>>>,
    progress: Option<DevProgressCallback>,
    toolchain_id: &str,
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
            progress(DevOperationProgress {
                toolchain_id: toolchain_id.to_owned(),
                phase: "running",
                stream: stream.to_owned(),
                message: sanitized,
            });
        }
    }
}

fn manager_capture(
    toolchain: &DevelopmentToolchain,
    home: &Path,
    data_dir: &Path,
    args: &[String],
) -> Result<String, String> {
    let output = manager_command(toolchain, home, data_dir, args)?
        .output()
        .map_err(|error| format!("无法执行 {}：{error}", toolchain.display_name))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn manager_command(
    toolchain: &DevelopmentToolchain,
    home: &Path,
    data_dir: &Path,
    args: &[String],
) -> Result<Command, String> {
    match toolchain.manager_kind {
        ManagerKind::Shell => {
            let script = toolchain
                .manager_script
                .as_deref()
                .ok_or_else(|| "该版本管理器未配置可 source 的脚本".to_owned())?;
            let script_path = data_dir.join(script);
            if !script_path.is_file() {
                return Err(format!("版本管理器脚本缺失：{}", script_path.display()));
            }
            let mut parts = vec![format!(
                "source {} --no-use >/dev/null 2>&1; nvm",
                shell_quote(&script_path.to_string_lossy())
            )];
            for argument in args {
                parts.push(shell_quote(argument));
            }
            let mut command = Command::new("/bin/bash");
            command
                .arg("-c")
                .arg(parts.join(" "))
                .env_clear()
                .env("PATH", SAFE_SYSTEM_PATH)
                .env("HOME", home)
                .env("NVM_DIR", data_dir)
                .env("LC_ALL", "C")
                .env("LANG", "C")
                .env("LANGUAGE", "C");
            apply_proxy_environment(&mut command);
            Ok(command)
        }
        ManagerKind::Binary => {
            let binary = resolve_binary(toolchain).ok_or_else(|| format!("未找到 {} 可执行文件", toolchain.manager))?;
            let mut command = Command::new(&binary);
            command
                .args(args)
                .env_clear()
                .env("PATH", SAFE_SYSTEM_PATH)
                .env("HOME", home)
                .env("RUSTUP_HOME", data_dir)
                .env("LC_ALL", "C")
                .env("LANG", "C")
                .env("LANGUAGE", "C");
            apply_proxy_environment(&mut command);
            Ok(command)
        }
    }
}

fn apply_proxy_environment(command: &mut Command) {
    for (key, value) in crate::network::proxy_environment() {
        command.env(key, value);
    }
}

fn resolve_data_dir(toolchain: &DevelopmentToolchain) -> Option<PathBuf> {
    match toolchain.manager_kind {
        ManagerKind::Shell => {
            if toolchain.manager == "nvm"
                && let Some(directory) = std::env::var_os("NVM_DIR")
            {
                let candidate = PathBuf::from(directory);
                if toolchain
                    .manager_script
                    .as_ref()
                    .is_some_and(|script| candidate.join(script).is_file())
                {
                    return Some(candidate);
                }
            }
            let candidate = expand_path(&toolchain.manager_home).ok()?;
            candidate
                .join(toolchain.manager_script.as_deref()?)
                .is_file()
                .then_some(candidate)
        }
        ManagerKind::Binary => {
            if let Some(directory) = std::env::var_os("RUSTUP_HOME") {
                let candidate = PathBuf::from(directory);
                if candidate.is_dir() {
                    return Some(candidate);
                }
            }
            expand_path(&toolchain.manager_home).ok().filter(|dir| dir.is_dir())
        }
    }
}

fn resolve_binary(toolchain: &DevelopmentToolchain) -> Option<PathBuf> {
    if let Some(binary) = &toolchain.manager_binary {
        if let Ok(path) = expand_path(binary) {
            if path.is_file() {
                return Some(path);
            }
        }
    }
    find_on_path(&toolchain.manager)
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

fn shorten_toolchain(name: &str) -> String {
    name.split('-').next().unwrap_or(name).to_owned()
}

fn parse_manager_version(toolchain: &DevelopmentToolchain, line: &str) -> String {
    match toolchain.manager_kind {
        ManagerKind::Shell => line.trim().to_owned(),
        ManagerKind::Binary => line.split_whitespace().nth(1).unwrap_or(line).to_owned(),
    }
}

fn channel_key(toolchain: &str) -> (u8, Vec<u64>) {
    let rank = match toolchain {
        "stable" => 0,
        "beta" => 1,
        "nightly" => 2,
        _ => 3,
    };
    (rank, version_key(toolchain).into_iter().rev().collect())
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

fn is_version(value: &str) -> bool {
    let components: Vec<_> = value.split('.').collect();
    components.len() == 3
        && components
            .iter()
            .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_concrete_version(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(is_version)
}

fn version_key(value: &str) -> Vec<u64> {
    value
        .trim_start_matches('v')
        .split('.')
        .map(|component| component.parse().unwrap_or(0))
        .collect()
}

fn validate_version_token(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('-')
        || value.contains("..")
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '/' | '*'))
    {
        return Err("版本或别名格式无效".to_owned());
    }
    Ok(())
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
    fn parses_lts_lines() {
        assert_eq!(
            parse_lts_line("       v24.18.1   (LTS: Krypton)"),
            Some(("v24.18.1".to_owned(), 24, "Krypton".to_owned(), false))
        );
        assert_eq!(
            parse_lts_line("->     v24.19.0 * (Latest LTS: Krypton)"),
            Some(("v24.19.0".to_owned(), 24, "Krypton".to_owned(), true))
        );
        assert_eq!(parse_lts_line("       v24.18.1"), None);
    }

    #[test]
    fn selects_one_recommended_lts_release() {
        let output = "       v22.23.2   (LTS: Jod)\n->     v24.19.0 * (Latest LTS: Krypton)";
        let releases = parse_lts_releases(output);
        assert_eq!(releases.len(), 2);
        assert_eq!(releases.iter().filter(|item| item.recommended).count(), 1);
        assert_eq!(releases.iter().find(|item| item.version == "v24.19.0").unwrap().recommended, true);
        assert_eq!(releases.iter().find(|item| item.version == "v22.23.2").unwrap().label.as_deref(), Some("LTS Jod"));
    }

    #[test]
    fn validates_version_tokens() {
        assert!(validate_version_token("v24.19.0").is_ok());
        assert!(validate_version_token("stable").is_ok());
        assert!(validate_version_token("1.75.0").is_ok());
        assert!(validate_version_token("--default").is_err());
        assert!(validate_version_token("../etc").is_err());
        assert!(validate_version_token("a'b").is_err());
    }

    #[test]
    fn sorts_versions_descending() {
        assert!(version_key("v24.19.0") > version_key("v24.2.0"));
        assert!(version_key("v24.19.0") < version_key("v24.19.1"));
    }

    #[test]
    fn shortens_toolchains_to_their_channel() {
        assert_eq!(shorten_toolchain("stable-x86_64-unknown-linux-gnu"), "stable");
        assert_eq!(shorten_toolchain("1.75.0-x86_64-unknown-linux-gnu"), "1.75.0");
    }

    #[test]
    fn rust_channels_are_offered() {
        let channels = rust_channels();
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0].version, "stable");
        assert!(channels[0].recommended);
    }

    #[test]
    fn embedded_toolchains_are_configured() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.development_toolchains.len(), 2);
        assert_eq!(catalog.by_toolchain_id("nodejs").unwrap().manager_kind, ManagerKind::Shell);
        assert_eq!(catalog.by_toolchain_id("rust").unwrap().manager_kind, ManagerKind::Binary);
    }

    #[test]
    #[ignore = "depends on rustup being installed on the host"]
    fn detects_local_rust_through_rustup() {
        let catalog = Catalog::load().unwrap();
        let rust = catalog.by_toolchain_id("rust").unwrap().clone();
        let state = detect_state_sync(&rust).unwrap();
        assert!(state.manager_found);
        assert!(state.installed_versions.iter().any(|item| item.version == "stable"));
        assert!(state.manager_version.is_some());
    }
}
