use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use umanager_catalog::{Catalog, DevelopmentToolchain};

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
    pub major: u32,
    pub lts: String,
    pub latest_lts: bool,
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
    let (toolchain, home, nvm_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || list_remote_versions_sync(&toolchain, &home, &nvm_dir))
        .await
        .map_err(|error| format!("远程版本读取任务异常结束：{error}"))?
}

pub async fn install_version(
    toolchain_id: String,
    version: String,
    progress: DevProgressCallback,
) -> Result<DevOperationReport, String> {
    validate_nvm_token(&version)?;
    let (toolchain, home, nvm_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let args = vec!["install".to_owned(), version.clone(), "--default".to_owned()];
        let output = run_nvm_streaming(&toolchain, &home, &nvm_dir, &args, Some(&progress))?;
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
    validate_nvm_token(&version)?;
    let (toolchain, home, nvm_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let args = vec!["alias".to_owned(), "default".to_owned(), version.clone()];
        let output = run_nvm_streaming(&toolchain, &home, &nvm_dir, &args, Some(&progress))?;
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
    validate_nvm_token(&version)?;
    let (toolchain, home, nvm_dir) = prepare(&toolchain_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let args = vec!["uninstall".to_owned(), version.clone()];
        let output = run_nvm_streaming(&toolchain, &home, &nvm_dir, &args, Some(&progress))?;
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
    let nvm_dir = resolve_manager_home(&toolchain)
        .ok_or_else(|| format!("未检测到 {}（{} 缺失）", toolchain.display_name, toolchain.manager_home))?;
    Ok((toolchain, home, nvm_dir))
}

fn detect_state_sync(toolchain: &DevelopmentToolchain) -> Result<DevToolchainState, String> {
    let home = user_home()?;
    let nvm_dir = resolve_manager_home(toolchain);
    let manager_found = nvm_dir.is_some();
    let manager_version = match nvm_dir.as_ref() {
        Some(dir) => nvm_capture(&home, dir, &toolchain.manager_script, &["--version".to_owned()])
            .ok()
            .filter(|value| !value.is_empty()),
        None => None,
    };
    let installed = nvm_dir
        .as_ref()
        .map(|dir| list_installed_versions(toolchain, dir))
        .unwrap_or_default();
    let lts_aliases = nvm_dir
        .as_ref()
        .map(|dir| lts_aliases(dir))
        .unwrap_or_default();
    let default_version = match nvm_dir.as_ref() {
        Some(dir) => nvm_capture(&home, dir, &toolchain.manager_script, &["version".to_owned(), "default".to_owned()])
            .ok()
            .filter(|value| !value.is_empty()),
        None => None,
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
    installed_versions.sort_by(|left, right| version_key(&right.version).cmp(&version_key(&left.version)));

    Ok(DevToolchainState {
        toolchain_id: toolchain.toolchain_id.clone(),
        display_name: toolchain.display_name.clone(),
        vendor: toolchain.vendor.clone(),
        homepage: toolchain.homepage.clone(),
        manager: toolchain.manager.clone(),
        manager_found,
        manager_home: nvm_dir.as_ref().map(|dir| dir.to_string_lossy().into_owned()),
        manager_version,
        default_version,
        installed_versions,
    })
}

fn list_installed_versions(toolchain: &DevelopmentToolchain, nvm_dir: &Path) -> Vec<String> {
    let directory = expand_path(&toolchain.versions_directory)
        .unwrap_or_else(|_| nvm_dir.join("versions").join("node"));
    let Ok(entries) = fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut versions = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (entry.path().is_dir() && name.starts_with('v') && name.len() > 1).then_some(name)
        })
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| version_key(right).cmp(&version_key(left)));
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
    nvm_dir: &Path,
) -> Result<Vec<DevRelease>, String> {
    let output = nvm_capture(
        home,
        nvm_dir,
        &toolchain.manager_script,
        &["ls-remote".to_owned(), "--lts".to_owned()],
    )?;
    let mut by_major: HashMap<u32, (String, String, bool)> = HashMap::new();
    for line in output.lines() {
        if let Some((version, major, codename, latest)) = parse_lts_line(line) {
            by_major.insert(major, (version, codename, latest));
        }
    }
    let mut releases = by_major
        .into_iter()
        .map(|(major, (version, lts, _latest))| DevRelease {
            version,
            major,
            lts,
            latest_lts: false,
        })
        .collect::<Vec<_>>();
    if let Some(latest) = releases
        .iter()
        .max_by(|left, right| {
            left.major.cmp(&right.major).then_with(|| version_key(&left.version).cmp(&version_key(&right.version)))
        })
        .map(|release| release.version.clone())
    {
        for release in &mut releases {
            release.latest_lts = release.version == latest;
        }
    }
    releases.sort_by(|left, right| right.major.cmp(&left.major));
    Ok(releases)
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

fn run_nvm_streaming(
    toolchain: &DevelopmentToolchain,
    home: &Path,
    nvm_dir: &Path,
    args: &[String],
    progress: Option<&DevProgressCallback>,
) -> Result<String, String> {
    let script = build_nvm_script(nvm_dir, &toolchain.manager_script, args)?;
    let mut command = nvm_command(home, nvm_dir);
    command
        .arg("-c")
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
        std::thread::spawn(move || {
            forward_lines(stdout, "stdout", &collected, progress, &toolchain_id)
        })
    };
    let stderr_thread = {
        let collected = Arc::clone(&collected);
        let progress = progress.cloned();
        let toolchain_id = toolchain_id.clone();
        std::thread::spawn(move || {
            forward_lines(stderr, "stderr", &collected, progress, &toolchain_id)
        })
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
            message: if status.success() {
                "操作已成功完成".to_owned()
            } else {
                "操作失败".to_owned()
            },
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

fn nvm_capture(
    home: &Path,
    nvm_dir: &Path,
    script: &str,
    args: &[String],
) -> Result<String, String> {
    let script_text = build_nvm_script(nvm_dir, script, args)?;
    let output = nvm_command(home, nvm_dir)
        .arg("-c")
        .arg(&script_text)
        .output()
        .map_err(|error| format!("无法执行 nvm：{error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn build_nvm_script(nvm_dir: &Path, script: &str, args: &[String]) -> Result<String, String> {
    let script_path = nvm_dir.join(script);
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
    Ok(parts.join(" "))
}

fn nvm_command(home: &Path, nvm_dir: &Path) -> Command {
    let mut command = Command::new("/bin/bash");
    command
        .env_clear()
        .env("PATH", SAFE_SYSTEM_PATH)
        .env("HOME", home)
        .env("NVM_DIR", nvm_dir)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    command
}

fn resolve_manager_home(toolchain: &DevelopmentToolchain) -> Option<PathBuf> {
    if toolchain.manager == "nvm"
        && let Some(directory) = std::env::var_os("NVM_DIR")
    {
        let candidate = PathBuf::from(directory);
        if candidate.join(&toolchain.manager_script).is_file() {
            return Some(candidate);
        }
    }
    let candidate = expand_path(&toolchain.manager_home).ok()?;
    candidate
        .join(&toolchain.manager_script)
        .is_file()
        .then_some(candidate)
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
    value
        .strip_prefix('v')
        .is_some_and(is_version)
}

fn version_key(value: &str) -> Vec<u64> {
    value
        .trim_start_matches('v')
        .split('.')
        .map(|component| component.parse().unwrap_or(0))
        .collect()
}

fn validate_nvm_token(value: &str) -> Result<(), String> {
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
    fn validates_nvm_tokens() {
        assert!(validate_nvm_token("v24.19.0").is_ok());
        assert!(validate_nvm_token("lts/*").is_ok());
        assert!(validate_nvm_token("--default").is_err());
        assert!(validate_nvm_token("../etc").is_err());
        assert!(validate_nvm_token("a'b").is_err());
    }

    #[test]
    fn sorts_versions_descending() {
        assert!(version_key("v24.19.0") > version_key("v24.2.0"));
        assert!(version_key("v24.19.0") < version_key("v24.19.1"));
    }

    #[test]
    fn embedded_toolchains_are_configured() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.development_toolchains.len(), 1);
        assert_eq!(catalog.by_toolchain_id("nodejs").unwrap().manager, "nvm");
    }
}
