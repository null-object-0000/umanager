use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const PACKAGE_NAME: &str = "u-manager";
pub const APPLICATION_ID: &str = "umanager";
const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallationKind {
    DebianPackage,
    Portable,
    Development,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationInfo {
    pub app_version: String,
    pub installation_kind: InstallationKind,
    pub package_name: Option<String>,
    pub package_version: Option<String>,
    pub architecture: Option<String>,
    pub executable_path: String,
    pub can_self_remove: bool,
}

pub fn detect() -> Result<InstallationInfo, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("无法确定 UManager 可执行文件位置：{error}"))?;
    let executable = normalize_executable_path(executable);
    let executable_path = executable.to_string_lossy().into_owned();
    let package = query_package_state();
    let owned = package
        .as_ref()
        .is_some_and(|_| package_owns_path(&executable).unwrap_or(false));

    let installation_kind = if owned {
        InstallationKind::DebianPackage
    } else if cfg!(debug_assertions) {
        InstallationKind::Development
    } else {
        InstallationKind::Portable
    };

    Ok(InstallationInfo {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        installation_kind,
        package_name: owned.then(|| PACKAGE_NAME.to_owned()),
        package_version: owned
            .then(|| package.as_ref().map(|item| item.0.clone()))
            .flatten(),
        architecture: owned
            .then(|| package.as_ref().map(|item| item.1.clone()))
            .flatten(),
        executable_path,
        can_self_remove: owned,
    })
}

fn query_package_state() -> Option<(String, String)> {
    let output = clean_command(DPKG_QUERY_BIN)
        .args([
            "-W",
            "-f=${db:Status-Abbrev}\t${Version}\t${Architecture}",
            PACKAGE_NAME,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_package_state(String::from_utf8_lossy(&output.stdout).trim_end())
}

fn parse_package_state(rendered: &str) -> Option<(String, String)> {
    let mut fields = rendered.split('\t');
    let status = fields.next()?;
    let version = fields.next()?;
    let architecture = fields.next()?;
    (fields.next().is_none() && status == "ii " && !version.is_empty() && !architecture.is_empty())
        .then(|| (version.to_owned(), architecture.to_owned()))
}

fn package_owns_path(path: &Path) -> Result<bool, String> {
    let output = clean_command(DPKG_QUERY_BIN)
        .args(["-L", PACKAGE_NAME])
        .output()
        .map_err(|error| format!("无法读取 UManager 安装清单：{error}"))?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|entry| Path::new(entry) == path))
}

/// On Linux, when a running binary's on-disk file is replaced — as `dpkg --install`
/// does during a UManager self-update — `/proc/self/exe` resolves to
/// `<path> (deleted)`. Strip that suffix so the path still refers to the real
/// on-disk binary: it must match the file that `dpkg-query -L` reports and be the
/// path relaunched after an update. Without this, the updated process would be
/// detected as "portable" and relaunching `… (deleted)` would fail with ENOENT.
#[cfg(target_os = "linux")]
fn normalize_executable_path(path: PathBuf) -> PathBuf {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    const DELETED_SUFFIX: &[u8] = b" (deleted)";
    let raw = path.as_os_str().as_bytes();
    if raw.ends_with(DELETED_SUFFIX) {
        PathBuf::from(OsStr::from_bytes(&raw[..raw.len() - DELETED_SUFFIX.len()]))
    } else {
        path
    }
}

#[cfg(not(target_os = "linux"))]
fn normalize_executable_path(path: PathBuf) -> PathBuf {
    path
}

fn clean_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", SAFE_SYSTEM_PATH)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_a_fully_installed_package() {
        assert_eq!(
            parse_package_state("ii \t0.1.0\tamd64"),
            Some(("0.1.0".to_owned(), "amd64".to_owned()))
        );
        assert_eq!(parse_package_state("rc \t0.1.0\tamd64"), None);
        assert_eq!(parse_package_state("ii \t0.1.0"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn strips_deleted_suffix_from_executable_path() {
        assert_eq!(
            normalize_executable_path(PathBuf::from("/usr/bin/umanager (deleted)")),
            PathBuf::from("/usr/bin/umanager")
        );
        assert_eq!(
            normalize_executable_path(PathBuf::from("/usr/bin/umanager")),
            PathBuf::from("/usr/bin/umanager")
        );
    }
}
