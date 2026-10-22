//! Conservative cleanup shared by the app and privileged helper. System packages
//! come from the installed dependency graph intersected with APT's autoremove
//! simulation, never from plan-provided package names alone. User paths are a
//! compiled allowlist and are removed ONLY by the unprivileged app.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

// Device/inode values can exceed JavaScript's exact integer range.
mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupPackage {
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserEntry {
    /// Relative to the requesting user's home; never used by root.
    pub path: String,
    pub bytes: u64,
    #[serde(with = "decimal_u64")]
    pub device: u64,
    #[serde(with = "decimal_u64")]
    pub inode: u64,
    pub modified_seconds: i64,
    pub modified_nanos: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemEntry {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupSelection {
    pub packages: Vec<CleanupPackage>,
    pub user_entries: Vec<UserEntry>,
    #[serde(default)]
    pub system_entries: Vec<SystemEntry>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupScan {
    pub selection: CleanupSelection,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
struct Installed {
    version: String,
    dependencies: Vec<String>,
    protected: bool,
}

fn command(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn package_token(value: &str) -> bool {
    let mut parts = value.split(':');
    let name = parts.next().unwrap_or_default();
    super::valid_package_name(name)
        && parts.clone().count() <= 1
        && parts.all(|arch| {
            !arch.is_empty() && arch.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn dependency_names(value: &str) -> Vec<String> {
    value
        .split([',', '|'])
        .filter_map(|part| {
            let name = part.trim().split([' ', '(']).next()?;
            package_token(name).then(|| name.to_owned())
        })
        .collect()
}

fn installed_packages(text: &str) -> BTreeMap<String, Installed> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 6 || fields[1] != "installed" || !package_token(fields[0]) {
                return None;
            }
            Some((
                fields[0].to_owned(),
                Installed {
                    version: fields[2].to_owned(),
                    dependencies: dependency_names(fields[3]),
                    protected: fields[4] == "yes" || fields[5] == "yes",
                },
            ))
        })
        .collect()
}

/// Only accept removals, never installs/upgrades/configurations or malformed
/// action lines. Locale is fixed to C by the caller.
fn simulated_removals(output: &str) -> Result<BTreeSet<String>, String> {
    let mut removed = BTreeSet::new();
    for line in output.lines() {
        if line.starts_with("Inst ") || line.starts_with("Conf ") {
            return Err("APT 预演需要安装或修改其他软件，拒绝清理".to_owned());
        }
        if line.starts_with("Remv ") || line.starts_with("Purg ") {
            let name = line.split_whitespace().nth(1).unwrap_or_default();
            if !package_token(name) {
                return Err("APT 预演返回了无效包名".to_owned());
            }
            removed.insert(name.to_owned());
        }
    }
    Ok(removed)
}

fn resolve_name(name: &str, installed: &BTreeMap<String, Installed>) -> Option<String> {
    if installed.contains_key(name) {
        return Some(name.to_owned());
    }
    // Resolve a native package's optional architecture suffix only when unique.
    if name.contains(':') && !name.ends_with(":any") && !name.ends_with(":native") {
        return None;
    }
    let base = name.split(':').next()?;
    let matches: Vec<_> = installed
        .keys()
        .filter(|key| key.split(':').next() == Some(base))
        .collect();
    (matches.len() == 1).then(|| matches[0].clone())
}

fn dependency_closure(target: &str, installed: &BTreeMap<String, Installed>) -> BTreeSet<String> {
    let mut visited = BTreeSet::new();
    let mut pending = vec![target.to_owned()];
    while let Some(name) = pending.pop() {
        let Some(name) = resolve_name(&name, installed) else {
            continue;
        };
        if !visited.insert(name.clone()) {
            continue;
        }
        pending.extend(installed[&name].dependencies.clone());
    }
    if let Some(target) = resolve_name(target, installed) {
        visited.remove(&target);
    }
    visited
}

fn read_installed() -> Result<BTreeMap<String, Installed>, String> {
    Ok(installed_packages(&command(
        "/usr/bin/dpkg-query",
        &[
            "-W",
            "-f=${binary:Package}\t${db:Status-Status}\t${Version}\t${Pre-Depends}, ${Depends}, ${Recommends}\t${Essential}\t${Protected}\n",
        ],
    )?))
}

pub fn scan_dependencies(target: &str) -> Result<Vec<CleanupPackage>, String> {
    if !super::valid_package_name(target) {
        return Err("无效目标包名".to_owned());
    }
    let installed = read_installed()?;
    let automatic: BTreeSet<_> = command("/usr/bin/apt-mark", &["showauto"])?
        .lines()
        .filter_map(|name| resolve_name(name, &installed))
        .collect();
    let simulation = command(
        "/usr/bin/apt-get",
        &["-s", "--autoremove", "purge", "--", target],
    )?;
    let removals = simulated_removals(&simulation)?;
    Ok(eligible_packages(target, &installed, &automatic, &removals))
}

fn eligible_packages(
    target: &str,
    installed: &BTreeMap<String, Installed>,
    automatic: &BTreeSet<String>,
    removals: &BTreeSet<String>,
) -> Vec<CleanupPackage> {
    let closure = dependency_closure(target, installed);
    removals
        .iter()
        .filter_map(|name| resolve_name(name, installed))
        .filter(|name| {
            closure.contains(name) && automatic.contains(name) && !installed[name].protected
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|name| CleanupPackage {
            version: installed[&name].version.clone(),
            name,
        })
        .collect()
}

/// Re-derive eligibility independently in the helper, then simulate ONLY the
/// selected fixed list. This rejects reverse-dependent removals and shared deps.
/// Actual execution is dpkg --purge, so no solver can expand the approved list.
pub fn validate_packages(target: &str, selected: &[CleanupPackage]) -> Result<(), String> {
    let eligible = scan_dependencies(target)?;
    let mut names = BTreeSet::from([target.to_owned()]);
    for package in selected {
        if !eligible.contains(package) || !names.insert(package.name.clone()) {
            return Err(format!(
                "依赖状态已变化或不允许清理：{}；请重新扫描",
                package.name
            ));
        }
    }
    let mut args = vec!["-s", "purge", "--"];
    args.extend(names.iter().map(String::as_str));
    let installed = read_installed()?;
    let removals = simulated_removals(&command("/usr/bin/apt-get", &args)?)?
        .into_iter()
        .map(|name| resolve_name(&name, &installed).unwrap_or(name))
        .collect::<BTreeSet<_>>();
    let names = names
        .into_iter()
        .map(|name| resolve_name(&name, &installed).unwrap_or(name))
        .collect::<BTreeSet<_>>();
    if removals != names {
        return Err("APT 预演与确认清单不一致，拒绝额外移除软件；请重新扫描".to_owned());
    }
    Ok(())
}

/// System cleanup is limited to reviewed, regular root-owned Docker repository
/// files. Never remove a shared repository while another Docker component stays.
const DOCKER_SOURCE_FILES: &[&str] = &[
    "/etc/apt/sources.list.d/docker.list",
    "/etc/apt/sources.list.d/docker.sources",
    "/etc/apt/keyrings/docker.asc",
    "/etc/apt/keyrings/docker.gpg",
];

fn docker_sources_exclusive(target: &str, packages: &[CleanupPackage]) -> Result<bool, String> {
    if target != "docker-desktop" {
        return Ok(false);
    }
    let states = command(
        "/usr/bin/dpkg-query",
        &["-W", "-f=${binary:Package}\t${db:Status-Status}\n"],
    )?;
    Ok(!states.lines().any(|line| {
        let Some((name, status)) = line.split_once('\t') else {
            return true;
        };
        if matches!(status, "not-installed" | "config-files") {
            return false;
        }
        let base = name.split(':').next().unwrap_or(name);
        (base.starts_with("docker")
            || matches!(
                base,
                "containerd.io" | "containerd" | "moby-engine" | "moby-cli"
            ))
            && base != target
            && !packages.iter().any(|p| p.name == name)
    }))
}

fn read_system_entry(path: &Path) -> Result<SystemEntry, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.uid() != 0 || meta.mode() & 0o022 != 0 || meta.len() > 1024 * 1024 {
        return Err("仅允许清理 root 所有、不可由普通用户写入的小型普通文件".to_owned());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(SystemEntry {
        path: path.to_string_lossy().into_owned(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes: meta.len(),
    })
}

pub fn scan_system(target: &str, packages: &[CleanupPackage]) -> Result<Vec<SystemEntry>, String> {
    if !docker_sources_exclusive(target, packages)? {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for path in DOCKER_SOURCE_FILES {
        if fs::symlink_metadata(path).is_ok() {
            entries.push(read_system_entry(Path::new(path))?);
        }
    }
    Ok(entries)
}

fn validate_repository_keys(entries: &[SystemEntry]) -> Result<(), String> {
    let keys: Vec<_> = entries
        .iter()
        .filter(|entry| entry.path.starts_with("/etc/apt/keyrings/"))
        .collect();
    if keys.is_empty() {
        return Ok(());
    }
    let mut sources = vec![PathBuf::from("/etc/apt/sources.list")];
    for entry in fs::read_dir("/etc/apt/sources.list.d").map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("list" | "sources")
        ) {
            sources.push(path);
        }
    }
    for path in sources {
        if entries.iter().any(|entry| Path::new(&entry.path) == path) {
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("无法核对软件源 {}：{e}", path.display())),
        };
        if keys.iter().any(|key| content.contains(&key.path)) {
            return Err(format!(
                "保留的软件源 {} 仍引用所选密钥，请取消密钥清理或同时选择对应软件源",
                path.display()
            ));
        }
    }
    Ok(())
}

pub fn validate_system_entries(target: &str, selection: &CleanupSelection) -> Result<(), String> {
    if selection.system_entries.is_empty() {
        return Ok(());
    }
    if !docker_sources_exclusive(target, &selection.packages)? {
        return Err(
            "Docker 软件源仍被保留的组件使用，不能删除；请重新扫描或取消软件源清理".to_owned(),
        );
    }
    validate_repository_keys(&selection.system_entries)?;
    let mut seen = BTreeSet::new();
    for entry in &selection.system_entries {
        if !DOCKER_SOURCE_FILES.contains(&entry.path.as_str()) || !seen.insert(&entry.path) {
            return Err("系统文件不在编译期清理白名单中".to_owned());
        }
        let current = read_system_entry(Path::new(&entry.path))?;
        if current != *entry {
            return Err(format!("系统文件已变化，请重新扫描：{}", entry.path));
        }
    }
    Ok(())
}

/// Called ONLY after helper catalog authorization and package-list validation.
/// No plan-supplied path reaches filesystem removal without an exact allowlist.
pub fn remove_system_entries(target: &str, selection: &CleanupSelection) -> Vec<String> {
    // Vendor purge scripts may already have removed one of the approved files.
    // Missing files count as cleaned; changed or substituted files do not.
    let mut remaining = selection.clone();
    for entry in &selection.system_entries {
        if !DOCKER_SOURCE_FILES.contains(&entry.path.as_str()) {
            return vec!["系统文件不在编译期清理白名单中".to_owned()];
        }
    }
    remaining.system_entries.retain(|entry| {
        !matches!(
            fs::symlink_metadata(&entry.path), Err(e) if e.kind() == std::io::ErrorKind::NotFound
        )
    });
    if let Err(error) = validate_system_entries(target, &remaining) {
        return vec![error];
    }
    let mut failures = Vec::new();
    for entry in &remaining.system_entries {
        if let Err(error) = fs::remove_file(&entry.path) {
            failures.push(format!("{}：{error}", entry.path));
        }
    }
    failures
}

/// No guesses from display names, arbitrary feed paths or package-name prefixes.
/// Add app-specific paths here after review. Unknown apps still get exact cached
/// .deb detection; their unknown personal data is explicitly reported as unscanned.
fn profile(package: &str) -> &'static [&'static str] {
    match package {
        "docker-desktop" => &[
            ".docker",
            ".config/Docker Desktop",
            ".config/docker-secrets-engine",
            ".cache/Docker Desktop",
            ".cache/docker-secrets-engine",
        ],
        "code" => &[".config/Code", ".vscode", ".cache/Code"],
        "google-chrome-stable" => &[".config/google-chrome", ".cache/google-chrome"],
        "microsoft-edge-stable" => &[".config/microsoft-edge", ".cache/microsoft-edge"],
        _ => &[],
    }
}

fn allowed_user_path(package: &str, relative: &str) -> bool {
    if profile(package).contains(&relative) {
        return true;
    }
    let prefix = ".cache/io.github.umanager.app/downloads/";
    let Some(file) = relative.strip_prefix(prefix) else {
        return false;
    };
    let Some(hash) = file
        .strip_prefix(&format!("{package}-"))
        .and_then(|s| s.strip_suffix(".deb"))
    else {
        return false;
    };
    // source_engine currently uses 16 hex SHA-256 chars in cache filenames.
    hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Anchor each ancestor to a held directory fd, O_NOFOLLOW at every step. Later
/// operations use /proc/self/fd so swapping a home subdirectory for a symlink
/// cannot redirect scanning/deletion into unrelated user files.
fn anchored_parent(home: &Path, relative: &str) -> Result<(File, PathBuf), String> {
    let path = Path::new(relative);
    if !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err("清理路径必须是 home 下的相对路径".to_owned());
    }
    let open_dir = |p: &Path| {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(p)
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let mut parent = open_dir(home)?;
    for component in path.parent().unwrap_or(Path::new("")).components() {
        parent = open_dir(
            &PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd())).join(component),
        )?;
    }
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()))
        .join(path.file_name().ok_or("清理路径为空")?);
    Ok((parent, anchored))
}

fn mount_id(directory: &File) -> Result<u64, String> {
    let info = fs::read_to_string(format!("/proc/self/fdinfo/{}", directory.as_raw_fd()))
        .map_err(|e| e.to_string())?;
    info.lines()
        .find_map(|line| line.strip_prefix("mnt_id:")?.trim().parse().ok())
        .ok_or_else(|| "无法核对目录挂载身份".to_owned())
}

fn measure(
    path: &Path,
    device: u64,
    expected_mount: u64,
    remaining: &mut usize,
    depth: usize,
) -> Result<u64, String> {
    if depth > 128 {
        return Err("目录层级过深，未纳入自动清理".to_owned());
    }
    if *remaining == 0 {
        return Err("目录超过 100000 项，未纳入自动清理".to_owned());
    }
    *remaining -= 1;
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.dev() != device {
        return Err("目录包含其他挂载点，未纳入自动清理".to_owned());
    }
    if meta.is_dir() {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)
            .map_err(|e| e.to_string())?;
        if mount_id(&directory)? != expected_mount {
            return Err("目录含挂载点，未纳入自动清理".to_owned());
        }
        let base = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let mut bytes = meta.blocks() * 512;
        for entry in fs::read_dir(&base).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            bytes = bytes.saturating_add(measure(
                &base.join(entry.file_name()),
                device,
                expected_mount,
                remaining,
                depth + 1,
            )?);
        }
        Ok(bytes)
    } else {
        Ok(meta.blocks() * 512)
    }
}

pub fn scan_user(home: &Path, package: &str) -> CleanupScan {
    let mut scan = CleanupScan::default();
    let mut paths: Vec<_> = profile(package).iter().map(|s| s.to_string()).collect();
    if profile(package).is_empty() {
        scan.warnings.push("该软件尚无经过审核的个人数据清理规则，仅扫描 UManager 安装包缓存；其他目录需手动检查。".to_owned());
    }
    let downloads = ".cache/io.github.umanager.app/downloads";
    // Hold its ancestor fd while enumerating. Do not follow a symlink cache dir.
    if let Ok((_parent, path)) = anchored_parent(home, &format!("{downloads}/placeholder")) {
        if let Some(parent) = path.parent() {
            if let Ok(entries) = fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let relative = format!("{downloads}/{}", entry.file_name().to_string_lossy());
                    if allowed_user_path(package, &relative) {
                        paths.push(relative);
                    }
                }
            }
        }
    }
    for relative in paths {
        let result = (|| {
            let (parent, anchored) = anchored_parent(home, &relative)?;
            let meta = match fs::symlink_metadata(&anchored) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e.to_string()),
            };
            let bytes = measure(&anchored, meta.dev(), mount_id(&parent)?, &mut 100_000, 0)?;
            Ok(Some(UserEntry {
                path: relative.clone(),
                bytes,
                device: meta.dev(),
                inode: meta.ino(),
                modified_seconds: meta.mtime(),
                modified_nanos: meta.mtime_nsec(),
            }))
        })();
        match result {
            Ok(Some(entry)) => scan.selection.user_entries.push(entry),
            Ok(None) => (),
            Err(error) => scan.warnings.push(format!("未扫描 {relative}：{error}")),
        }
    }
    scan
}

pub fn validate_user_entries(
    home: &Path,
    package: &str,
    entries: &[UserEntry],
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        if !allowed_user_path(package, &entry.path) || !seen.insert(&entry.path) {
            return Err("清理清单含未授权或重复的个人路径".to_owned());
        }
        let (_parent, path) = anchored_parent(home, &entry.path)?;
        match fs::symlink_metadata(&path) {
            Ok(meta)
                if meta.dev() == entry.device
                    && meta.ino() == entry.inode
                    && meta.mtime() == entry.modified_seconds
                    && meta.mtime_nsec() == entry.modified_nanos =>
            {
                ()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err(format!("目录已变化，请重新扫描：{}", entry.path)),
        }
    }
    Ok(())
}

/// Delete through held directory fds, checking mount identity at every descent.
/// This also rejects same-device bind mounts (st_dev alone cannot detect them).
fn remove_tree(
    path: &Path,
    device: u64,
    expected_mount: u64,
    remaining: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 {
        return Err("清理目录层级过深".to_owned());
    }
    if *remaining == 0 {
        return Err("清理项目超过限制".to_owned());
    }
    *remaining -= 1;
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.dev() != device {
        return Err("清理时遇到其他挂载点".to_owned());
    }
    if !meta.is_dir() {
        return fs::remove_file(path).map_err(|e| e.to_string());
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())?;
    if mount_id(&directory)? != expected_mount {
        return Err("清理时遇到挂载点".to_owned());
    }
    let base = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    for entry in fs::read_dir(&base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        remove_tree(
            &base.join(entry.file_name()),
            device,
            expected_mount,
            remaining,
            depth + 1,
        )?;
    }
    fs::remove_dir(path).map_err(|e| e.to_string())
}

/// Runs without privilege. Returns per-path failures; package removal may already
/// have succeeded, so never misreport a partial cleanup as complete.
pub fn remove_user_entries(home: &Path, package: &str, entries: &[UserEntry]) -> Vec<String> {
    let mut failures = Vec::new();
    for entry in entries {
        let result = (|| {
            validate_user_entries(home, package, std::slice::from_ref(entry))?;
            let (parent, path) = anchored_parent(home, &entry.path)?;
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e.to_string()),
            };
            // Recheck mounts and readability immediately before removal.
            let expected_mount = mount_id(&parent)?;
            measure(&path, meta.dev(), expected_mount, &mut 100_000, 0)?;
            remove_tree(&path, meta.dev(), expected_mount, &mut 100_000, 0)
        })();
        if let Err(error) = result {
            failures.push(format!("{}：{error}", entry.path));
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inode_identity_round_trips_without_javascript_precision_loss() {
        let entry = UserEntry {
            path: ".docker".into(),
            bytes: 1,
            device: u64::MAX,
            inode: 9_007_199_254_740_999,
            modified_seconds: 1,
            modified_nanos: 2,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["inode"], "9007199254740999");
        assert_eq!(serde_json::from_value::<UserEntry>(json).unwrap(), entry);
    }
    #[test]
    fn scope_excludes_unrelated_orphans_and_walks_alternatives() {
        let installed = installed_packages(
            "app\tinstalled\t1\tlib | other\t\t\nlib\tinstalled\t2\tbase\t\t\nother\tinstalled\t1\t\t\t\nbase\tinstalled\t3\t\t\t\nunrelated\tinstalled\t1\t\t\t\nremoved\tconfig-files\t1\t\t\t\n",
        );
        assert_eq!(
            dependency_closure("app", &installed),
            BTreeSet::from(["lib".into(), "other".into(), "base".into()])
        );
        assert!(!installed.contains_key("removed"));
        assert!(simulated_removals("Inst surprise [1]\n").is_err());
        assert!(simulated_removals("Purg --evil\n").is_err());
    }
    #[test]
    fn candidates_require_all_three_conditions_and_exclude_protected_packages() {
        let mut installed = BTreeMap::new();
        for name in [
            "app",
            "unused",
            "shared",
            "manual",
            "essential",
            "unrelated",
        ] {
            installed.insert(
                name.to_owned(),
                Installed {
                    version: "1".into(),
                    dependencies: vec![],
                    protected: name == "essential",
                },
            );
        }
        installed.get_mut("app").unwrap().dependencies = vec![
            "unused".into(),
            "shared".into(),
            "manual".into(),
            "essential".into(),
        ];
        let automatic = ["unused", "shared", "essential", "unrelated"]
            .map(str::to_owned)
            .into_iter()
            .collect();
        let removals = ["app", "unused", "manual", "essential", "unrelated"]
            .map(str::to_owned)
            .into_iter()
            .collect();
        assert_eq!(
            eligible_packages("app", &installed, &automatic, &removals),
            vec![CleanupPackage {
                name: "unused".into(),
                version: "1".into()
            }]
        );
    }
    #[test]
    fn multiarch_resolution_never_substitutes_another_architecture() {
        let installed = installed_packages("lib:amd64\tinstalled\t1\t\t\t\n");
        assert_eq!(resolve_name("lib", &installed), Some("lib:amd64".into()));
        assert_eq!(resolve_name("lib:i386", &installed), None);
        let protected = installed_packages("base\tinstalled\t1\t\tyes\t\n");
        assert!(protected["base"].protected);
    }
    #[test]
    fn system_files_cannot_be_supplied_for_unrelated_targets() {
        let forged = CleanupSelection {
            system_entries: vec![SystemEntry {
                path: "/etc/passwd".into(),
                sha256: "0".repeat(64),
                bytes: 1,
            }],
            ..Default::default()
        };
        assert!(validate_system_entries("wechat", &forged).is_err());
    }
    #[test]
    fn only_exact_compiled_paths_and_cache_hashes() {
        assert!(allowed_user_path("docker-desktop", ".docker"));
        for path in [
            ".docker/../.ssh",
            ".config",
            "/home/me/.docker",
            ".cache/io.github.umanager.app/downloads/docker-desktop-../../.deb",
        ] {
            assert!(!allowed_user_path("docker-desktop", path));
        }
        assert!(!allowed_user_path("unknown", ".config/unknown"));
        assert!(allowed_user_path(
            "docker-desktop",
            ".cache/io.github.umanager.app/downloads/docker-desktop-0123456789abcdef.deb"
        ));
    }
    #[test]
    fn mount_mismatch_and_nested_symlinks_do_not_delete_outside_data() {
        use std::os::unix::fs::symlink;
        let home =
            std::env::temp_dir().join(format!("umanager-cleanup-mount-{}", std::process::id()));
        fs::create_dir_all(home.join(".docker/nested")).unwrap();
        fs::create_dir_all(home.join("outside")).unwrap();
        fs::write(home.join("outside/keep"), "safe").unwrap();
        symlink(home.join("outside"), home.join(".docker/nested/link")).unwrap();
        let (parent, path) = anchored_parent(&home, ".docker").unwrap();
        let meta = fs::symlink_metadata(&path).unwrap();
        let mount = mount_id(&parent).unwrap();
        assert!(remove_tree(&path, meta.dev(), mount + 1, &mut 100_000, 0).is_err());
        assert!(home.join(".docker/nested/link").is_symlink());
        let scan = scan_user(&home, "docker-desktop");
        assert!(
            remove_user_entries(&home, "docker-desktop", &scan.selection.user_entries).is_empty()
        );
        assert!(home.join("outside/keep").exists());
        assert!(!home.join(".docker").exists());
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn symlink_ancestors_and_replaced_directories_are_rejected() {
        use std::os::unix::fs::symlink;
        let home = std::env::temp_dir().join(format!("umanager-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("outside")).unwrap();
        fs::write(home.join("outside/keep"), "safe").unwrap();
        symlink(home.join("outside"), home.join(".config")).unwrap();
        assert!(anchored_parent(&home, ".config/Code").is_err());
        symlink(home.join("outside"), home.join(".docker")).unwrap();
        let scan = scan_user(&home, "docker-desktop");
        let entry = scan
            .selection
            .user_entries
            .iter()
            .find(|e| e.path == ".docker")
            .unwrap();
        assert!(
            remove_user_entries(&home, "docker-desktop", std::slice::from_ref(entry)).is_empty()
        );
        assert!(home.join("outside/keep").exists());
        fs::create_dir(home.join(".docker")).unwrap();
        assert!(
            validate_user_entries(&home, "docker-desktop", std::slice::from_ref(entry)).is_err()
        );
        fs::remove_dir_all(home).unwrap();
    }
}
