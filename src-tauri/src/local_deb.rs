use crate::operation_plan::{PlanArtifact, persist_immutable_plan};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use umanager_plan::{
    MAX_PLAN_LIFETIME_SECONDS, OperationAction, OperationPlan, PLAN_SCHEMA_VERSION, PlanPayload,
};

const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";
const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const DPKG_BIN: &str = "/usr/bin/dpkg";
const MAX_DEB_SIZE: u64 = 4 * 1024 * 1024 * 1024;

pub struct LocalDebState {
    launch_path: Mutex<Option<PathBuf>>,
}

impl LocalDebState {
    pub fn from_process_arguments() -> Self {
        Self {
            launch_path: Mutex::new(find_deb_argument(std::env::args_os().skip(1))),
        }
    }

    pub fn pending_path(&self) -> Result<Option<PathBuf>, String> {
        self.launch_path
            .lock()
            .map(|path| path.clone())
            .map_err(|_| "本地安装包状态不可用".to_owned())
    }

    pub fn from_path_for_command(path: PathBuf) -> Self {
        Self {
            launch_path: Mutex::new(Some(path)),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDebInspection {
    original_path: String,
    cached_path: Option<String>,
    file_name: String,
    package_name: String,
    version: String,
    architecture: String,
    size: u64,
    sha256: String,
    installed_version: Option<String>,
    disposition: InstallDisposition,
    install_allowed: bool,
    source_trusted: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum InstallDisposition {
    NewInstall,
    Upgrade,
    Reinstall,
    Downgrade,
    UnsupportedArchitecture,
}

pub fn inspect_pending(state: &LocalDebState) -> Result<Option<LocalDebInspection>, String> {
    state
        .pending_path()?
        .map(|path| inspect(&path, None))
        .transpose()
}

pub fn import_pending(
    state: &LocalDebState,
    cache_dir: &Path,
) -> Result<LocalDebInspection, String> {
    let source_path = state
        .pending_path()?
        .ok_or_else(|| "UManager 不是通过本地 .deb 启动的".to_owned())?;
    let preview = inspect(&source_path, None)?;
    if !preview.install_allowed {
        return Err(disposition_error(preview.disposition).to_owned());
    }

    let imports_dir = cache_dir.join("imports");
    fs::create_dir_all(&imports_dir).map_err(|error| format!("无法创建导入缓存目录：{error}"))?;
    let target = imports_dir.join(format!("{}.deb", preview.sha256));
    copy_verified(&source_path, &target, preview.size, &preview.sha256)?;
    inspect(&target, Some(target.clone()))
}

pub fn create_plan(cache_dir: &Path, sha256: &str) -> Result<PlanArtifact, String> {
    validate_sha256(sha256)?;
    let cached_path = cache_dir.join("imports").join(format!("{sha256}.deb"));
    let inspected = inspect(&cached_path, Some(cached_path.clone()))?;
    if !inspected.install_allowed {
        return Err(disposition_error(inspected.disposition).to_owned());
    }
    let created = unix_timestamp();
    let plan = OperationPlan::new(PlanPayload {
        schema_version: PLAN_SCHEMA_VERSION,
        action: OperationAction::InstallLocalDeb,
        application_id: format!("local-deb:{}", inspected.package_name),
        package_name: inspected.package_name,
        installed_version: inspected.installed_version,
        target_version: inspected.version,
        architecture: inspected.architecture,
        deb_path: cached_path.to_string_lossy().into_owned(),
        sha256: inspected.sha256,
        size: inspected.size,
        created_at_unix_seconds: created,
        expires_at_unix_seconds: created + MAX_PLAN_LIFETIME_SECONDS,
        catalog_json: None,
        catalog_signature: None,
    })?;
    let path = persist_immutable_plan(&cache_dir.join("plans"), &plan)?;
    Ok(PlanArtifact {
        plan,
        plan_path: path.to_string_lossy().into_owned(),
    })
}

fn inspect(path: &Path, cached_path: Option<PathBuf>) -> Result<LocalDebInspection, String> {
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("无法读取本地安装包：{error}"))?;
    let metadata =
        fs::metadata(&canonical).map_err(|error| format!("无法检查本地安装包：{error}"))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_DEB_SIZE {
        return Err("本地安装包必须是小于 4 GiB 的普通非空文件".to_owned());
    }
    let package_name = deb_field(&canonical, "Package")?;
    let version = deb_field(&canonical, "Version")?;
    let architecture = deb_field(&canonical, "Architecture")?;
    validate_debian_field("包名", &package_name)?;
    validate_debian_field("版本", &version)?;
    validate_debian_field("架构", &architecture)?;
    let system_architecture = command_output(DPKG_BIN, &["--print-architecture"])?;
    let installed_version = installed_version(&package_name);
    let disposition = if architecture != system_architecture && architecture != "all" {
        InstallDisposition::UnsupportedArchitecture
    } else if let Some(installed) = installed_version.as_deref() {
        if debian_relation(installed, "eq", &version) {
            InstallDisposition::Reinstall
        } else if debian_relation(installed, "lt", &version) {
            InstallDisposition::Upgrade
        } else {
            InstallDisposition::Downgrade
        }
    } else {
        InstallDisposition::NewInstall
    };
    let sha256 = hash_file(&canonical)?;
    Ok(LocalDebInspection {
        original_path: canonical.to_string_lossy().into_owned(),
        cached_path: cached_path.map(|path| path.to_string_lossy().into_owned()),
        file_name: canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("package.deb")
            .to_owned(),
        package_name,
        version,
        architecture,
        size: metadata.len(),
        sha256,
        installed_version,
        disposition,
        install_allowed: matches!(
            disposition,
            InstallDisposition::NewInstall | InstallDisposition::Upgrade
        ),
        source_trusted: false,
    })
}

fn copy_verified(
    source_path: &Path,
    target: &Path,
    expected_size: u64,
    expected_hash: &str,
) -> Result<(), String> {
    if target.exists() {
        let existing =
            fs::metadata(target).map_err(|error| format!("无法检查已有缓存：{error}"))?;
        if existing.is_file()
            && existing.len() == expected_size
            && hash_file(target)? == expected_hash
        {
            return Ok(());
        }
        return Err("同名缓存文件与导入内容不一致".to_owned());
    }
    let mut source =
        File::open(source_path).map_err(|error| format!("无法打开本地安装包：{error}"))?;
    let mut target_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)
        .map_err(|error| format!("无法创建导入缓存：{error}"))?;
    let result = (|| {
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = source
                .read(&mut buffer)
                .map_err(|error| format!("读取本地安装包失败：{error}"))?;
            if read == 0 {
                break;
            }
            size += read as u64;
            if size > expected_size {
                return Err("导入期间源文件发生变化".to_owned());
            }
            hasher.update(&buffer[..read]);
            target_file
                .write_all(&buffer[..read])
                .map_err(|error| format!("写入导入缓存失败：{error}"))?;
        }
        target_file
            .sync_all()
            .map_err(|error| format!("同步导入缓存失败：{error}"))?;
        let hash = format!("{:x}", hasher.finalize());
        if size != expected_size || hash != expected_hash {
            return Err("导入期间源文件内容发生变化".to_owned());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(target);
    }
    result
}

fn find_deb_argument<I>(arguments: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = OsString>,
{
    arguments.into_iter().map(PathBuf::from).find(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("deb"))
    })
}

fn deb_field(path: &Path, field: &str) -> Result<String, String> {
    let output = clean_command(DPKG_DEB_BIN)
        .arg("--field")
        .arg(path)
        .arg(field)
        .output()
        .map_err(|error| format!("无法读取 .deb {field} 字段：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "不是有效的 Debian 安装包：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.is_empty() {
        return Err(format!("Debian 安装包缺少 {field} 字段"));
    }
    Ok(value)
}

fn installed_version(package_name: &str) -> Option<String> {
    command_output(
        DPKG_QUERY_BIN,
        &["-W", "-f=${db:Status-Abbrev}\t${Version}", package_name],
    )
    .ok()
    .and_then(|value| value.strip_prefix("ii \t").map(str::to_owned))
}

fn debian_relation(left: &str, relation: &str, right: &str) -> bool {
    clean_command(DPKG_BIN)
        .args(["--compare-versions", left, relation, right])
        .status()
        .is_ok_and(|status| status.success())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("无法读取安装包：{error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("无法计算安装包哈希：{error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("本地安装包 SHA-256 无效".to_owned());
    }
    Ok(())
}

fn validate_debian_field(label: &str, value: &str) -> Result<(), String> {
    if value.len() > 256 || value.contains('\0') || value.chars().any(char::is_control) {
        return Err(format!("Debian 安装包{label}字段无效"));
    }
    Ok(())
}

fn command_output(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = clean_command(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("无法执行 {program}：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} 执行失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn clean_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    command
}

fn disposition_error(disposition: InstallDisposition) -> &'static str {
    match disposition {
        InstallDisposition::Reinstall => "当前版本相同，拒绝重装",
        InstallDisposition::Downgrade => "本地安装包版本更旧，拒绝降级",
        InstallDisposition::UnsupportedArchitecture => "本地安装包架构与当前系统不兼容",
        _ => "本地安装包当前不可安装",
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_fixture() -> (PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("umanager-local-deb-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        let package_root = root.join("package");
        let control_dir = package_root.join("DEBIAN");
        fs::create_dir_all(&control_dir).unwrap();
        fs::write(
            control_dir.join("control"),
            "Package: umanager-test-local\nVersion: 1.0\nArchitecture: all\nMaintainer: UManager Tests <test@example.invalid>\nDescription: Safe empty local package fixture\n",
        )
        .unwrap();
        let deb_path = root.join("umanager-test-local_1.0_all.deb");
        let status = Command::new(DPKG_DEB_BIN)
            .args(["--root-owner-group", "--build"])
            .arg(&package_root)
            .arg(&deb_path)
            .status()
            .unwrap();
        assert!(status.success());
        (root, deb_path)
    }

    #[test]
    fn accepts_only_deb_launch_arguments() {
        assert_eq!(
            find_deb_argument([OsString::from("/tmp/app.deb")]),
            Some(PathBuf::from("/tmp/app.deb"))
        );
        assert_eq!(find_deb_argument([OsString::from("/tmp/app.txt")]), None);
    }

    #[test]
    fn rejects_path_like_hashes() {
        assert!(validate_sha256("../package").is_err());
        assert!(validate_sha256(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn inspects_imports_and_plans_an_empty_fixture_without_installing_it() {
        let (root, deb_path) = build_fixture();
        let cache = root.join("cache");
        let state = LocalDebState::from_path_for_command(deb_path);
        let preview = inspect_pending(&state).unwrap().unwrap();
        assert_eq!(preview.package_name, "umanager-test-local");
        assert_eq!(preview.version, "1.0");
        assert_eq!(preview.architecture, "all");
        assert!(preview.install_allowed);

        let imported = import_pending(&state, &cache).unwrap();
        assert!(imported.cached_path.is_some());
        let artifact = create_plan(&cache, &imported.sha256).unwrap();
        assert_eq!(
            artifact.plan.payload.action,
            OperationAction::InstallLocalDeb
        );
        assert_eq!(artifact.plan.payload.package_name, "umanager-test-local");
        assert!(artifact.plan.verify_integrity().is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
