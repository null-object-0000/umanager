use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use umanager_plan::{OperationAction, OperationPlan};

const PACKAGE_NAME: &str = "code";
const ARCHITECTURE: &str = "amd64";
const APT_CACHE_BIN: &str = "/usr/bin/apt-cache";
const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const DPKG_BIN: &str = "/usr/bin/dpkg";
const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";

#[derive(Debug)]
struct Invocation {
    plan_path: PathBuf,
    expected_action: OperationAction,
    mode: ExecutionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecutionMode {
    DryRun,
    Execute,
}

#[derive(Debug)]
struct Caller {
    uid: u32,
    home: PathBuf,
}

#[derive(Debug)]
struct DebMetadata {
    package: String,
    version: String,
    architecture: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationReport {
    dry_run: bool,
    plan_id: String,
    action: OperationAction,
    package_name: String,
    installed_version: Option<String>,
    target_version: String,
    architecture: String,
    sha256: String,
    verified: bool,
    system_modified: bool,
    planned_command: PlannedCommand,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedCommand {
    executable: &'static str,
    arguments: Vec<String>,
}

fn main() {
    match run() {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("serialize dry-run report")
            );
        }
        Err(error) => {
            eprintln!("umanager-helper: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<OperationReport, String> {
    let invocation = parse_invocation(std::env::args_os().skip(1))?;
    ensure_running_as_root()?;
    let caller = resolve_caller()?;
    let now = unix_timestamp();
    let (plan, cache_root) = load_and_validate_plan(&invocation.plan_path, &caller, now)?;
    if plan.payload.action != invocation.expected_action {
        return Err("operation plan action does not match the requested helper action".to_owned());
    }
    match plan.payload.action {
        OperationAction::InstallVerifiedDeb => {
            validate_vscode_constraints(&plan)?;
            validate_installed_version(&plan)?;
            validate_official_apt_record(&plan)?;
        }
        OperationAction::InstallLocalDeb => validate_local_constraints(&plan)?,
    }

    let staged_path = stage_verified_deb(&plan, &cache_root, caller.uid)?;
    let validation = (|| {
        let metadata = inspect_deb(&staged_path)?;
        validate_deb_metadata(&plan, &metadata)
    })();
    validation?;
    let planned_command = fixed_install_command(&staged_path);
    let execution = if invocation.mode == ExecutionMode::Execute {
        execute_fixed_install(&staged_path).map(|()| true)
    } else {
        Ok(false)
    };
    let cleanup = fs::remove_file(&staged_path)
        .map_err(|error| format!("cannot remove privileged staging file: {error}"));
    let system_modified = execution?;
    cleanup?;

    Ok(OperationReport {
        dry_run: invocation.mode == ExecutionMode::DryRun,
        plan_id: plan.plan_id,
        action: plan.payload.action,
        package_name: plan.payload.package_name,
        installed_version: plan.payload.installed_version,
        target_version: plan.payload.target_version,
        architecture: plan.payload.architecture,
        sha256: plan.payload.sha256,
        verified: true,
        system_modified,
        planned_command,
    })
}

fn parse_invocation<I>(arguments: I) -> Result<Invocation, String>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let arguments: Vec<_> = arguments.into_iter().collect();
    if arguments.len() != 4 || arguments[1] != "--plan" {
        return Err("invalid helper invocation".to_owned());
    }
    let (expected_action, mode) = match (
        arguments[0].to_string_lossy().as_ref(),
        arguments[3].to_string_lossy().as_ref(),
    ) {
        ("install-verified-deb", "--dry-run") => {
            (OperationAction::InstallVerifiedDeb, ExecutionMode::DryRun)
        }
        ("install-local-deb", "--dry-run") => {
            (OperationAction::InstallLocalDeb, ExecutionMode::DryRun)
        }
        ("install-local-deb", "--execute") => {
            (OperationAction::InstallLocalDeb, ExecutionMode::Execute)
        }
        _ => return Err("helper action or mode is not allowed".to_owned()),
    };
    let plan_path = PathBuf::from(&arguments[2]);
    if !plan_path.is_absolute() {
        return Err("operation plan path must be absolute".to_owned());
    }
    Ok(Invocation {
        plan_path,
        expected_action,
        mode,
    })
}

fn ensure_running_as_root() -> Result<(), String> {
    let status = fs::read_to_string("/proc/self/status")
        .map_err(|error| format!("cannot read process credentials: {error}"))?;
    let effective_uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|values| values.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| "cannot determine effective uid".to_owned())?;
    if effective_uid != 0 {
        return Err("helper must be launched through Polkit as root".to_owned());
    }
    Ok(())
}

fn resolve_caller() -> Result<Caller, String> {
    let uid = std::env::var("PKEXEC_UID")
        .map_err(|_| "PKEXEC_UID is missing; direct root execution is refused".to_owned())?
        .parse::<u32>()
        .map_err(|_| "PKEXEC_UID is invalid".to_owned())?;
    if uid == 0 {
        return Err("root cannot be the requesting desktop user".to_owned());
    }
    let passwd = fs::read_to_string("/etc/passwd")
        .map_err(|error| format!("cannot read /etc/passwd: {error}"))?;
    let home = passwd
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(':').collect();
            (fields.len() >= 7 && fields[2].parse::<u32>().ok() == Some(uid))
                .then(|| PathBuf::from(fields[5]))
        })
        .next()
        .ok_or_else(|| "requesting user does not have a passwd entry".to_owned())?;
    if !home.is_absolute() {
        return Err("requesting user's home directory is invalid".to_owned());
    }
    Ok(Caller { uid, home })
}

fn load_and_validate_plan(
    requested_path: &Path,
    caller: &Caller,
    now: u64,
) -> Result<(OperationPlan, PathBuf), String> {
    let cache_root = caller.home.join(".cache/io.github.umanager.app");
    let canonical_cache = fs::canonicalize(&cache_root)
        .map_err(|error| format!("cannot resolve UManager cache directory: {error}"))?;
    let canonical_plans = fs::canonicalize(canonical_cache.join("plans"))
        .map_err(|error| format!("cannot resolve UManager plan directory: {error}"))?;
    let canonical_plan = fs::canonicalize(requested_path)
        .map_err(|error| format!("cannot resolve operation plan: {error}"))?;
    if !canonical_plan.starts_with(&canonical_plans) {
        return Err("operation plan is outside the UManager plan directory".to_owned());
    }
    validate_user_file(&canonical_plan, caller.uid, true)?;
    let bytes = fs::read(&canonical_plan)
        .map_err(|error| format!("cannot read operation plan: {error}"))?;
    if bytes.len() > 64 * 1024 {
        return Err("operation plan is unexpectedly large".to_owned());
    }
    let plan: OperationPlan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("operation plan JSON is invalid: {error}"))?;
    plan.verify_integrity()?;
    plan.validate_time(now)?;
    let expected_name = format!("{}.json", plan.plan_id);
    if canonical_plan.file_name().and_then(|name| name.to_str()) != Some(&expected_name) {
        return Err("operation plan filename does not match its identifier".to_owned());
    }
    Ok((plan, canonical_cache))
}

fn validate_user_file(path: &Path, uid: u32, require_read_only: bool) -> Result<(), String> {
    let metadata = fs::metadata(path).map_err(|error| format!("cannot inspect file: {error}"))?;
    if !metadata.is_file() || metadata.uid() != uid {
        return Err("file is not a regular file owned by the requesting user".to_owned());
    }
    if require_read_only && metadata.permissions().mode() & 0o222 != 0 {
        return Err("operation plan must not have writable permission bits".to_owned());
    }
    Ok(())
}

fn validate_vscode_constraints(plan: &OperationPlan) -> Result<(), String> {
    let payload = &plan.payload;
    if payload.action != OperationAction::InstallVerifiedDeb
        || payload.application_id != "vscode"
        || payload.package_name != PACKAGE_NAME
        || payload.architecture != ARCHITECTURE
    {
        return Err("operation plan is not an allowed VS Code amd64 update".to_owned());
    }
    Ok(())
}

fn validate_installed_version(plan: &OperationPlan) -> Result<(), String> {
    let installed = command_output(DPKG_QUERY_BIN, &["-W", "-f=${Version}", PACKAGE_NAME])?;
    if plan.payload.installed_version.as_deref() != Some(&installed) {
        return Err("installed VS Code version changed after plan creation".to_owned());
    }
    let status = clean_command(DPKG_BIN)
        .args([
            "--compare-versions",
            &installed,
            "lt",
            &plan.payload.target_version,
        ])
        .status()
        .map_err(|error| format!("cannot compare Debian versions: {error}"))?;
    if !status.success() {
        return Err("target version is not newer; downgrade or reinstall is refused".to_owned());
    }
    Ok(())
}

fn validate_local_constraints(plan: &OperationPlan) -> Result<(), String> {
    if plan.payload.action != OperationAction::InstallLocalDeb
        || plan.payload.application_id != format!("local-deb:{}", plan.payload.package_name)
        || !valid_package_name(&plan.payload.package_name)
    {
        return Err("operation plan is not an allowed local Debian package install".to_owned());
    }
    let system_architecture = command_output(DPKG_BIN, &["--print-architecture"])?;
    if plan.payload.architecture != system_architecture && plan.payload.architecture != "all" {
        return Err("local package architecture is not supported by this system".to_owned());
    }

    let installed = command_output(
        DPKG_QUERY_BIN,
        &[
            "-W",
            "-f=${db:Status-Abbrev}\t${Version}",
            &plan.payload.package_name,
        ],
    )
    .ok()
    .and_then(|value| value.strip_prefix("ii \t").map(str::to_owned));
    if installed != plan.payload.installed_version {
        return Err("installed package state changed after plan creation".to_owned());
    }
    if let Some(installed_version) = installed {
        let status = clean_command(DPKG_BIN)
            .args([
                "--compare-versions",
                &installed_version,
                "lt",
                &plan.payload.target_version,
            ])
            .status()
            .map_err(|error| format!("cannot compare Debian versions: {error}"))?;
        if !status.success() {
            return Err("local package is not newer; reinstall or downgrade is refused".to_owned());
        }
    }
    Ok(())
}

fn valid_package_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')
        })
}

fn validate_official_apt_record(plan: &OperationPlan) -> Result<(), String> {
    let policy = command_output(APT_CACHE_BIN, &["policy", PACKAGE_NAME])?;
    let official_origin_present = policy.lines().any(|line| {
        line.split_whitespace()
            .any(|token| token.starts_with("https://packages.microsoft.com/repos/code"))
    });
    if !official_origin_present || !policy.contains(&plan.payload.target_version) {
        return Err(
            "target version is not backed by the configured Microsoft repository".to_owned(),
        );
    }

    let selector = format!("{PACKAGE_NAME}={}", plan.payload.target_version);
    let output = command_output(APT_CACHE_BIN, &["show", &selector])?;
    let fields = parse_control_fields(&output);
    if fields.get("Package").map(String::as_str) != Some(PACKAGE_NAME)
        || fields.get("Version") != Some(&plan.payload.target_version)
        || fields.get("Architecture").map(String::as_str) != Some(ARCHITECTURE)
        || fields.get("SHA256") != Some(&plan.payload.sha256)
        || fields.get("Size").and_then(|value| value.parse().ok()) != Some(plan.payload.size)
    {
        return Err("operation plan does not match the system's verified APT index".to_owned());
    }
    Ok(())
}

fn stage_verified_deb(
    plan: &OperationPlan,
    cache_root: &Path,
    caller_uid: u32,
) -> Result<PathBuf, String> {
    let cache_subdirectory = match plan.payload.action {
        OperationAction::InstallVerifiedDeb => "downloads",
        OperationAction::InstallLocalDeb => "imports",
    };
    let canonical_downloads = fs::canonicalize(cache_root.join(cache_subdirectory))
        .map_err(|error| format!("cannot resolve UManager package cache directory: {error}"))?;
    let deb_path = fs::canonicalize(&plan.payload.deb_path)
        .map_err(|error| format!("cannot resolve downloaded package: {error}"))?;
    if !deb_path.starts_with(&canonical_downloads) {
        return Err("downloaded package is outside the UManager cache".to_owned());
    }
    if plan.payload.action == OperationAction::InstallLocalDeb {
        let expected_name = format!("{}.deb", plan.payload.sha256);
        if deb_path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
            return Err("local package cache filename does not match its SHA-256".to_owned());
        }
    }
    let mut source = File::open(&deb_path)
        .map_err(|error| format!("cannot open downloaded package: {error}"))?;
    let source_metadata = source
        .metadata()
        .map_err(|error| format!("cannot inspect downloaded package: {error}"))?;
    if !source_metadata.is_file()
        || source_metadata.uid() != caller_uid
        || source_metadata.len() != plan.payload.size
    {
        return Err("downloaded package owner, size, or file type is invalid".to_owned());
    }

    let staged_path = PathBuf::from(format!(
        "/var/tmp/.umanager-{}-{}.deb",
        plan.plan_id,
        std::process::id()
    ));
    let mut staged = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged_path)
        .map_err(|error| format!("cannot create privileged staging file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|error| format!("cannot read downloaded package: {error}"))?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        if copied > plan.payload.size {
            let _ = fs::remove_file(&staged_path);
            return Err("downloaded package grew during privileged staging".to_owned());
        }
        hasher.update(&buffer[..count]);
        if let Err(error) = staged.write_all(&buffer[..count]) {
            let _ = fs::remove_file(&staged_path);
            return Err(format!("cannot write privileged staging file: {error}"));
        }
    }
    if let Err(error) = staged.sync_all() {
        let _ = fs::remove_file(&staged_path);
        return Err(format!("cannot sync privileged staging file: {error}"));
    }
    drop(staged);
    let digest = format!("{:x}", hasher.finalize());
    if copied != plan.payload.size || digest != plan.payload.sha256 {
        let _ = fs::remove_file(&staged_path);
        return Err("downloaded package size or SHA-256 does not match the plan".to_owned());
    }
    Ok(staged_path)
}

fn inspect_deb(path: &Path) -> Result<DebMetadata, String> {
    Ok(DebMetadata {
        package: read_deb_field(path, "Package")?,
        version: read_deb_field(path, "Version")?,
        architecture: read_deb_field(path, "Architecture")?,
    })
}

fn read_deb_field(path: &Path, field: &str) -> Result<String, String> {
    let output = clean_command(DPKG_DEB_BIN)
        .arg("--field")
        .arg(path)
        .arg(field)
        .output()
        .map_err(|error| format!("cannot read .deb field {field}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot read .deb field {field}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn validate_deb_metadata(plan: &OperationPlan, metadata: &DebMetadata) -> Result<(), String> {
    if metadata.package != plan.payload.package_name
        || metadata.version != plan.payload.target_version
        || metadata.architecture != plan.payload.architecture
    {
        return Err("staged .deb metadata does not match the operation plan".to_owned());
    }
    Ok(())
}

fn fixed_install_command(staged_path: &Path) -> PlannedCommand {
    PlannedCommand {
        executable: DPKG_BIN,
        arguments: vec![
            "--install".to_owned(),
            staged_path.to_string_lossy().into_owned(),
        ],
    }
}

fn execute_fixed_install(staged_path: &Path) -> Result<(), String> {
    let output = clean_command(DPKG_BIN)
        .arg("--install")
        .arg(staged_path)
        .output()
        .map_err(|error| format!("cannot execute dpkg install: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "dpkg install failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn parse_control_fields(input: &str) -> std::collections::HashMap<String, String> {
    input
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn command_output(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = clean_command(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("cannot execute {program}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
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

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn accepts_only_the_exact_dry_run_invocation() {
        let arguments = [
            "install-verified-deb",
            "--plan",
            "/tmp/plan.json",
            "--dry-run",
        ]
        .map(OsString::from);
        assert!(parse_invocation(arguments).is_ok());
        assert!(parse_invocation([OsString::from("--help")]).is_err());
        assert!(
            parse_invocation(
                ["install-verified-deb", "--plan", "/tmp/plan.json"].map(OsString::from)
            )
            .is_err()
        );
        let local_execute =
            ["install-local-deb", "--plan", "/tmp/plan.json", "--execute"].map(OsString::from);
        let invocation = parse_invocation(local_execute).unwrap();
        assert_eq!(invocation.expected_action, OperationAction::InstallLocalDeb);
        assert_eq!(invocation.mode, ExecutionMode::Execute);
    }

    #[test]
    fn validates_only_the_vscode_amd64_action() {
        let payload = umanager_plan::PlanPayload {
            schema_version: umanager_plan::PLAN_SCHEMA_VERSION,
            action: OperationAction::InstallVerifiedDeb,
            application_id: "vscode".to_owned(),
            package_name: "code".to_owned(),
            installed_version: Some("1.0".to_owned()),
            target_version: "2.0".to_owned(),
            architecture: "amd64".to_owned(),
            deb_path: "/cache/code.deb".to_owned(),
            sha256: "a".repeat(64),
            size: 1,
            created_at_unix_seconds: 1,
            expires_at_unix_seconds: 2,
        };
        let mut plan = OperationPlan::new(payload).unwrap();
        assert!(validate_vscode_constraints(&plan).is_ok());
        plan.payload.architecture = "arm64".to_owned();
        assert!(validate_vscode_constraints(&plan).is_err());
    }

    #[test]
    fn parses_apt_control_fields_without_shelling_out() {
        let fields = parse_control_fields("Package: code\nVersion: 2.0\nSHA256: abc\n\n");
        assert_eq!(fields.get("Package").map(String::as_str), Some("code"));
        assert_eq!(fields.get("Version").map(String::as_str), Some("2.0"));
    }

    #[test]
    fn dry_run_exposes_only_the_fixed_dpkg_install_shape() {
        let command = fixed_install_command(Path::new("/var/tmp/.umanager-plan.deb"));
        assert_eq!(command.executable, "/usr/bin/dpkg");
        assert_eq!(
            command.arguments,
            ["--install", "/var/tmp/.umanager-plan.deb"]
        );
    }

    #[test]
    fn validates_debian_package_name_shape() {
        assert!(valid_package_name("example-app+edition.1"));
        assert!(!valid_package_name("-option"));
        assert!(!valid_package_name("Example"));
        assert!(!valid_package_name("name/other"));
    }
}
