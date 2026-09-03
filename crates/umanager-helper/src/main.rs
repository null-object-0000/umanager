use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use umanager_catalog::{Application, Catalog, SourceSpec};
use umanager_plan::{OperationAction, OperationPlan, RemovalAction, RemovalPlan};

const APT_CACHE_BIN: &str = "/usr/bin/apt-cache";
const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const DPKG_BIN: &str = "/usr/bin/dpkg";
const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";
const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
const UMANAGER_APPLICATION_ID: &str = "umanager";
const UMANAGER_PACKAGE_NAME: &str = "u-manager";
const LOG_EVENT_PREFIX: &str = "UMANAGER_EVENT\t";
const MAX_LOG_BYTES: usize = 256 * 1024;
const MAX_LOG_LINE_CHARS: usize = 2_000;
/// Ed25519 public key matching the CI `FEED_SIGNING_KEY` secret. Used only to
/// authorize feed-added applications (applications not in the embedded catalog).
const FEED_PUBLIC_KEY_HEX: &str = "57d369d3e46b3243073b4535673ffa784dc760e0f14d6d25fb04940b69b0c8f9";
const MAX_CATALOG_AUTH_BYTES: usize = 256 * 1024;
const MAX_PLAN_FILE_BYTES: usize = 512 * 1024;

fn load_catalog() -> Result<Catalog, String> {
    Catalog::load().map_err(|error| format!("内置软件源无效：{error}"))
}

#[derive(Clone, Debug)]
struct OfficialAptApplication {
    application_id: String,
    package_name: String,
    architecture: String,
    repository_url: String,
}

#[derive(Clone, Debug)]
struct ManagedApplication {
    application_id: String,
    package_name: String,
    architecture: String,
}

#[derive(Clone, Debug)]
struct WebsiteApplication {
    application_id: String,
    package_name: String,
    architecture: String,
}

fn official_apt_applications(catalog: &Catalog) -> Vec<OfficialAptApplication> {
    catalog
        .applications
        .iter()
        .filter_map(|application| match &application.source {
            SourceSpec::AptRepository { repository_url, .. } => Some(OfficialAptApplication {
                application_id: application.application_id.clone(),
                package_name: application.package_name.clone(),
                architecture: application.architecture.clone(),
                repository_url: repository_url.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn managed_applications(catalog: &Catalog) -> Vec<ManagedApplication> {
    catalog
        .applications
        .iter()
        .filter(|application| application.removable)
        .map(|application| ManagedApplication {
            application_id: application.application_id.clone(),
            package_name: application.package_name.clone(),
            architecture: application.architecture.clone(),
        })
        .collect()
}

fn website_applications(catalog: &Catalog) -> Vec<WebsiteApplication> {
    catalog
        .applications
        .iter()
        .filter(|application| application.is_auto_installable())
        .map(|application| WebsiteApplication {
            application_id: application.application_id.clone(),
            package_name: application.package_name.clone(),
            architecture: application.architecture.clone(),
        })
        .collect()
}

#[derive(Debug)]
struct Invocation {
    plan_path: PathBuf,
    expected_action: HelperAction,
    mode: ExecutionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelperAction {
    Install(OperationAction),
    Remove(RemovalAction),
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

#[derive(Debug, PartialEq, Eq)]
enum OfficialInstalledState {
    Installed {
        version: String,
        architecture: String,
    },
    NotInstalled,
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
struct RemovalReport {
    dry_run: bool,
    plan_id: String,
    action: RemovalAction,
    package_name: String,
    installed_version: String,
    architecture: String,
    verified: bool,
    system_modified: bool,
    planned_command: PlannedCommand,
}

#[derive(Serialize)]
#[serde(untagged)]
enum HelperReport {
    Installation(OperationReport),
    Removal(RemovalReport),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedCommand {
    executable: &'static str,
    arguments: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HelperProgressEvent<'a> {
    kind: &'a str,
    stream: &'a str,
    message: &'a str,
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

fn run() -> Result<HelperReport, String> {
    let invocation = parse_invocation(std::env::args_os().skip(1))?;
    ensure_running_as_root()?;
    let caller = resolve_caller()?;
    let now = unix_timestamp();
    let catalog = load_catalog()?;
    match invocation.expected_action {
        HelperAction::Install(expected_action) => {
            run_install(&invocation, expected_action, &caller, now, &catalog)
                .map(HelperReport::Installation)
        }
        HelperAction::Remove(expected_action) => {
            run_removal(&invocation, expected_action, &caller, now, &catalog)
                .map(HelperReport::Removal)
        }
    }
}

fn run_install(
    invocation: &Invocation,
    expected_action: OperationAction,
    caller: &Caller,
    now: u64,
    catalog: &Catalog,
) -> Result<OperationReport, String> {
    let (plan, cache_root) = load_and_validate_plan(&invocation.plan_path, &caller, now)?;
    if plan.payload.action != expected_action {
        return Err("operation plan action does not match the requested helper action".to_owned());
    }
    match plan.payload.action {
        OperationAction::InstallVerifiedDeb => {
            let application = validate_official_apt_constraints(&plan, catalog)?;
            validate_installed_version(&plan, &application)?;
            validate_official_apt_record(&plan, &application)?;
        }
        OperationAction::InstallVerifiedWebsiteDeb => {
            validate_website_constraints(&plan, catalog)?;
        }
        OperationAction::InstallLocalDeb => validate_local_constraints(&plan)?,
        OperationAction::InstallSelfUpdate => {
            validate_self_update_constraints(&plan, catalog)?;
        }
    }

    let staged_path = stage_verified_deb(&plan, &cache_root, caller.uid)?;
    let validation = (|| {
        let metadata = inspect_deb(&staged_path)?;
        validate_deb_metadata(&plan, &metadata)
    })();
    validation?;
    emit_progress("phase", "system", "特权 helper 已完成安装包与操作计划复核");
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

fn run_removal(
    invocation: &Invocation,
    expected_action: RemovalAction,
    caller: &Caller,
    now: u64,
    catalog: &Catalog,
) -> Result<RemovalReport, String> {
    let plan = load_and_validate_removal_plan(&invocation.plan_path, caller, now)?;
    if plan.payload.action != expected_action {
        return Err("removal plan action does not match the requested helper action".to_owned());
    }
    let application = validate_removal_constraints(&plan, catalog)?;
    validate_removal_installed_state(&plan, &application)?;
    emit_progress(
        "phase",
        "system",
        "特权 helper 已完成卸载计划与当前安装状态复核",
    );
    let planned_command = fixed_remove_command(&application.package_name);
    let system_modified = if invocation.mode == ExecutionMode::Execute {
        execute_fixed_remove(&application.package_name)?;
        true
    } else {
        false
    };
    Ok(RemovalReport {
        dry_run: invocation.mode == ExecutionMode::DryRun,
        plan_id: plan.plan_id,
        action: plan.payload.action,
        package_name: plan.payload.package_name,
        installed_version: plan.payload.installed_version,
        architecture: plan.payload.architecture,
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
        ("install-verified-deb", "--dry-run") => (
            HelperAction::Install(OperationAction::InstallVerifiedDeb),
            ExecutionMode::DryRun,
        ),
        ("install-verified-deb", "--execute") => (
            HelperAction::Install(OperationAction::InstallVerifiedDeb),
            ExecutionMode::Execute,
        ),
        ("install-local-deb", "--dry-run") => (
            HelperAction::Install(OperationAction::InstallLocalDeb),
            ExecutionMode::DryRun,
        ),
        ("install-verified-website-deb", "--dry-run") => (
            HelperAction::Install(OperationAction::InstallVerifiedWebsiteDeb),
            ExecutionMode::DryRun,
        ),
        ("install-verified-website-deb", "--execute") => (
            HelperAction::Install(OperationAction::InstallVerifiedWebsiteDeb),
            ExecutionMode::Execute,
        ),
        ("install-local-deb", "--execute") => (
            HelperAction::Install(OperationAction::InstallLocalDeb),
            ExecutionMode::Execute,
        ),
        ("install-umanager", "--dry-run") => (
            HelperAction::Install(OperationAction::InstallSelfUpdate),
            ExecutionMode::DryRun,
        ),
        ("install-umanager", "--execute") => (
            HelperAction::Install(OperationAction::InstallSelfUpdate),
            ExecutionMode::Execute,
        ),
        ("remove-managed-package", "--dry-run") => (
            HelperAction::Remove(RemovalAction::RemoveManagedPackage),
            ExecutionMode::DryRun,
        ),
        ("remove-managed-package", "--execute") => (
            HelperAction::Remove(RemovalAction::RemoveManagedPackage),
            ExecutionMode::Execute,
        ),
        ("remove-umanager", "--dry-run") => (
            HelperAction::Remove(RemovalAction::RemoveUmanager),
            ExecutionMode::DryRun,
        ),
        ("remove-umanager", "--execute") => (
            HelperAction::Remove(RemovalAction::RemoveUmanager),
            ExecutionMode::Execute,
        ),
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
    if bytes.len() > MAX_PLAN_FILE_BYTES {
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

fn load_and_validate_removal_plan(
    requested_path: &Path,
    caller: &Caller,
    now: u64,
) -> Result<RemovalPlan, String> {
    let cache_root = caller.home.join(".cache/io.github.umanager.app");
    let canonical_cache = fs::canonicalize(&cache_root)
        .map_err(|error| format!("cannot resolve UManager cache directory: {error}"))?;
    let canonical_plans = fs::canonicalize(canonical_cache.join("plans"))
        .map_err(|error| format!("cannot resolve UManager plan directory: {error}"))?;
    let canonical_plan = fs::canonicalize(requested_path)
        .map_err(|error| format!("cannot resolve removal plan: {error}"))?;
    if !canonical_plan.starts_with(&canonical_plans) {
        return Err("removal plan is outside the UManager plan directory".to_owned());
    }
    validate_user_file(&canonical_plan, caller.uid, true)?;
    let bytes =
        fs::read(&canonical_plan).map_err(|error| format!("cannot read removal plan: {error}"))?;
    if bytes.len() > MAX_PLAN_FILE_BYTES {
        return Err("removal plan is unexpectedly large".to_owned());
    }
    let plan: RemovalPlan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("removal plan JSON is invalid: {error}"))?;
    plan.verify_integrity()?;
    plan.validate_time(now)?;
    let expected_name = format!("{}.json", plan.plan_id);
    if canonical_plan.file_name().and_then(|name| name.to_str()) != Some(&expected_name) {
        return Err("removal plan filename does not match its identifier".to_owned());
    }
    Ok(plan)
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

fn validate_official_apt_constraints(
    plan: &OperationPlan,
    catalog: &Catalog,
) -> Result<OfficialAptApplication, String> {
    let payload = &plan.payload;
    official_apt_applications(catalog)
        .into_iter()
        .find(|application| {
            payload.action == OperationAction::InstallVerifiedDeb
                && payload.application_id == application.application_id
                && payload.package_name == application.package_name
                && payload.architecture == application.architecture
        })
        .ok_or_else(|| "operation plan is not an allowed official APT update".to_owned())
}

fn validate_removal_constraints(
    plan: &RemovalPlan,
    catalog: &Catalog,
) -> Result<ManagedApplication, String> {
    let payload = &plan.payload;
    match payload.action {
        RemovalAction::RemoveManagedPackage => {
            if let Some(application) = managed_applications(catalog)
                .into_iter()
                .find(|application| {
                    payload.application_id == application.application_id
                        && payload.package_name == application.package_name
                        && payload.architecture == application.architecture
                })
            {
                return Ok(application);
            }
            let application = authorize_feed_application(
                payload.catalog_json.as_deref(),
                payload.catalog_signature.as_deref(),
                payload.source_ref.as_ref(),
                payload.source_endorsement.as_deref(),
                payload.source_catalog_json.as_deref(),
                payload.source_catalog_signature.as_deref(),
                &payload.application_id,
                &payload.package_name,
                &payload.architecture,
                true,
            )?;
            Ok(ManagedApplication {
                application_id: application.application_id,
                package_name: application.package_name,
                architecture: application.architecture,
            })
        }
        RemovalAction::RemoveUmanager => (payload.application_id == UMANAGER_APPLICATION_ID
            && payload.package_name == UMANAGER_PACKAGE_NAME)
            .then_some(ManagedApplication {
                application_id: UMANAGER_APPLICATION_ID.to_owned(),
                package_name: UMANAGER_PACKAGE_NAME.to_owned(),
                architecture: "amd64".to_owned(),
            })
            .filter(|application| payload.architecture == application.architecture)
            .ok_or_else(|| {
                "self-removal plan does not exactly target the UManager package".to_owned()
            }),
    }
}

fn validate_removal_installed_state(
    plan: &RemovalPlan,
    application: &ManagedApplication,
) -> Result<(), String> {
    let installed = command_output(
        DPKG_QUERY_BIN,
        &[
            "-W",
            "-f=${db:Status-Abbrev}\t${Version}\t${Architecture}",
            application.package_name.as_str(),
        ],
    )?;
    if !installed_state_matches_removal_plan(&installed, plan, application) {
        return Err("installed package state changed after removal plan creation".to_owned());
    }
    Ok(())
}

fn installed_state_matches_removal_plan(
    installed: &str,
    plan: &RemovalPlan,
    application: &ManagedApplication,
) -> bool {
    let mut fields = installed.split('\t');
    let status = fields.next();
    let version = fields.next();
    let architecture = fields.next();
    fields.next().is_none()
        && status == Some("ii ")
        && version == Some(plan.payload.installed_version.as_str())
        && architecture == Some(application.architecture.as_str())
}

fn validate_installed_version(
    plan: &OperationPlan,
    application: &OfficialAptApplication,
) -> Result<(), String> {
    let state = query_official_installed_state(application.package_name.as_str())?;
    match (&plan.payload.installed_version, state) {
        (
            Some(expected_version),
            OfficialInstalledState::Installed {
                version,
                architecture,
            },
        ) if expected_version == &version && architecture == application.architecture => {
            let status = clean_command(DPKG_BIN)
                .args([
                    "--compare-versions",
                    &version,
                    "lt",
                    &plan.payload.target_version,
                ])
                .status()
                .map_err(|error| format!("cannot compare Debian versions: {error}"))?;
            if !status.success() {
                return Err(
                    "target version is not newer; downgrade or reinstall is refused".to_owned(),
                );
            }
        }
        (None, OfficialInstalledState::NotInstalled) => {
            let system_architecture = command_output(DPKG_BIN, &["--print-architecture"])?;
            if system_architecture != application.architecture {
                return Err(format!(
                    "{} new install requires an {} system",
                    application.package_name, application.architecture
                ));
            }
        }
        _ => return Err("installed package state changed after plan creation".to_owned()),
    }
    Ok(())
}

fn query_official_installed_state(package_name: &str) -> Result<OfficialInstalledState, String> {
    let output = clean_command(DPKG_QUERY_BIN)
        .args([
            "-W",
            "-f=${db:Status-Abbrev}\t${Version}\t${Architecture}",
            package_name,
        ])
        .output()
        .map_err(|error| format!("cannot query installed package state: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if output.status.code() == Some(1) && stderr.contains("no packages found matching") {
            return Ok(OfficialInstalledState::NotInstalled);
        }
        return Err(format!("dpkg-query failed: {}", stderr.trim()));
    }
    parse_official_installed_state(String::from_utf8_lossy(&output.stdout).trim_end())
}

fn parse_official_installed_state(rendered: &str) -> Result<OfficialInstalledState, String> {
    let mut fields = rendered.split('\t');
    let status = fields.next();
    let version = fields.next();
    let architecture = fields.next();
    if fields.next().is_some() || status.is_none() || version.is_none() || architecture.is_none() {
        return Err("dpkg-query returned an invalid package state".to_owned());
    }
    if status != Some("ii ") {
        return Ok(OfficialInstalledState::NotInstalled);
    }
    Ok(OfficialInstalledState::Installed {
        version: version.unwrap_or_default().to_owned(),
        architecture: architecture.unwrap_or_default().to_owned(),
    })
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

fn validate_self_update_constraints(plan: &OperationPlan, catalog: &Catalog) -> Result<(), String> {
    let source = catalog
        .self_update_source()
        .ok_or_else(|| "UManager self-update source is not configured".to_owned())?;
    if !self_update_plan_matches(plan, source) {
        return Err("operation plan is not an allowed UManager self-update".to_owned());
    }
    let installed = query_official_installed_state(source.package_name.as_str())?;
    match (&plan.payload.installed_version, installed) {
        (
            Some(expected_version),
            OfficialInstalledState::Installed {
                version,
                architecture,
            },
        ) => {
            if expected_version != &version || architecture != source.architecture {
                return Err("installed UManager state changed after plan creation".to_owned());
            }
            let status = clean_command(DPKG_BIN)
                .args([
                    "--compare-versions",
                    &version,
                    "lt",
                    &plan.payload.target_version,
                ])
                .status()
                .map_err(|error| format!("cannot compare UManager versions: {error}"))?;
            if !status.success() {
                return Err(
                    "self-update target is not newer; reinstall or downgrade is refused"
                        .to_owned(),
                );
            }
            Ok(())
        }
        (Some(_), _) => {
            Err("UManager is not installed as a Debian package; self-update is refused".to_owned())
        }
        (None, _) => Err("UManager self-update requires a currently installed Debian package"
            .to_owned()),
    }
}

fn self_update_plan_matches(
    plan: &OperationPlan,
    source: &umanager_catalog::SelfUpdateSource,
) -> bool {
    plan.payload.action == OperationAction::InstallSelfUpdate
        && plan.payload.application_id == source.application_id
        && plan.payload.package_name == source.package_name
        && plan.payload.architecture == source.architecture
}

fn validate_website_constraints(
    plan: &OperationPlan,
    catalog: &Catalog,
) -> Result<WebsiteApplication, String> {
    let application = website_application_for_plan(plan, catalog)?;
    let installed = query_official_installed_state(application.package_name.as_str())?;
    match (&plan.payload.installed_version, installed) {
        (
            Some(expected_version),
            OfficialInstalledState::Installed {
                version,
                architecture,
            },
        ) => {
            if expected_version != &version || architecture != application.architecture {
                return Err("installed package state changed after plan creation".to_owned());
            }
            let status = clean_command(DPKG_BIN)
                .args([
                    "--compare-versions",
                    &version,
                    "lt",
                    &plan.payload.target_version,
                ])
                .status()
                .map_err(|error| {
                    format!("cannot compare {} versions: {error}", application.package_name)
                })?;
            if !status.success() {
                return Err(
                    "website target is not newer; reinstall or downgrade is refused".to_owned(),
                );
            }
        }
        (None, OfficialInstalledState::NotInstalled) => {
            let system_architecture = command_output(DPKG_BIN, &["--print-architecture"])?;
            if system_architecture != application.architecture {
                return Err(format!(
                    "{} new install requires an {} system",
                    application.package_name, application.architecture
                ));
            }
        }
        _ => return Err("installed package state changed after plan creation".to_owned()),
    }
    Ok(application)
}

fn website_application_for_plan(
    plan: &OperationPlan,
    catalog: &Catalog,
) -> Result<WebsiteApplication, String> {
    let payload = &plan.payload;
    if payload.action != OperationAction::InstallVerifiedWebsiteDeb {
        return Err("operation plan is not an allowed official website update".to_owned());
    }
    if let Some(application) = website_applications(catalog)
        .into_iter()
        .find(|application| {
            payload.application_id == application.application_id
                && payload.package_name == application.package_name
                && payload.architecture == application.architecture
        })
    {
        return Ok(application);
    }
    let application = authorize_feed_application(
        payload.catalog_json.as_deref(),
        payload.catalog_signature.as_deref(),
        payload.source_ref.as_ref(),
        payload.source_endorsement.as_deref(),
        payload.source_catalog_json.as_deref(),
        payload.source_catalog_signature.as_deref(),
        &payload.application_id,
        &payload.package_name,
        &payload.architecture,
        false,
    )?;
    Ok(WebsiteApplication {
        application_id: application.application_id,
        package_name: application.package_name,
        architecture: application.architecture,
    })
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

/// Authorizes a feed-added application. Central-source apps verify the plan's
/// `catalog_json`+`catalog_signature` against the built-in central public key;
/// non-central-source apps go through the two-level chain (design §5).
fn authorize_feed_application(
    catalog_json: Option<&str>,
    catalog_signature: Option<&str>,
    source_ref: Option<&umanager_plan::SourceRef>,
    source_endorsement: Option<&str>,
    source_catalog_json: Option<&str>,
    source_catalog_signature: Option<&str>,
    application_id: &str,
    package_name: &str,
    architecture: &str,
    require_removable: bool,
) -> Result<Application, String> {
    if let Some(source_ref) = source_ref {
        feed_added_application_source(
            source_ref,
            source_endorsement,
            source_catalog_json,
            source_catalog_signature,
            application_id,
            package_name,
            architecture,
            require_removable,
            FEED_PUBLIC_KEY_HEX,
        )
    } else {
        feed_added_application(
            catalog_json,
            catalog_signature,
            application_id,
            package_name,
            architecture,
            require_removable,
        )
    }
}

/// Authorizes an application that is not part of the embedded catalog by
/// verifying the Ed25519 signature the plan carries over its signed catalog JSON.
fn feed_added_application(
    catalog_json: Option<&str>,
    catalog_signature: Option<&str>,
    application_id: &str,
    package_name: &str,
    architecture: &str,
    require_removable: bool,
) -> Result<Application, String> {
    let (Some(json), Some(signature)) = (catalog_json, catalog_signature) else {
        return Err("plan does not carry a signed catalog for this application".to_owned());
    };
    if json.len() > MAX_CATALOG_AUTH_BYTES || json.contains('\0') {
        return Err("signed catalog payload is invalid".to_owned());
    }
    verify_ed25519(json.as_bytes(), signature)?;
    let applications: Vec<Application> = serde_json::from_str(json)
        .map_err(|error| format!("signed catalog JSON is invalid: {error}"))?;
    applications
        .into_iter()
        .find(|application| {
            application.application_id == application_id
                && application.package_name == package_name
                && application.architecture == architecture
                && (!require_removable || application.removable)
        })
        .ok_or_else(|| "signed catalog does not authorize this application".to_owned())
}

/// Authorizes a feed-added application brought in by a non-central source. Two
/// independent verifications, then the same catalog lookup: the built-in central
/// key must endorse the reduced `source_ref` bytes (endorsement), and the
/// source's own key must sign the source catalog — only then is the application
/// accepted from that catalog (DESIGN-multi-source.md §5).
fn feed_added_application_source(
    source_ref: &umanager_plan::SourceRef,
    source_endorsement: Option<&str>,
    source_catalog_json: Option<&str>,
    source_catalog_signature: Option<&str>,
    application_id: &str,
    package_name: &str,
    architecture: &str,
    require_removable: bool,
    central_public_key_hex: &str,
) -> Result<Application, String> {
    let (Some(endorsement), Some(json), Some(signature)) =
        (source_endorsement, source_catalog_json, source_catalog_signature)
    else {
        return Err("plan does not carry a complete source chain for this application".to_owned());
    };
    if json.len() > MAX_CATALOG_AUTH_BYTES || json.contains('\0') {
        return Err("source catalog payload is invalid".to_owned());
    }
    // 1. The built-in central key must have endorsed this exact source reference
    //    (reduced `{sourceId, feedUrl, publicKeyHex}` bytes).
    let canonical = serde_json::to_vec(source_ref)
        .map_err(|error| format!("cannot canonicalize sourceRef: {error}"))?;
    verify_ed25519_with(&canonical, endorsement, central_public_key_hex)?;
    // 2. The source's own key must have signed its catalog.
    verify_ed25519_with(json.as_bytes(), signature, &source_ref.public_key_hex)?;
    // 3. The application must exist in the source's signed catalog.
    let applications: Vec<Application> = serde_json::from_str(json)
        .map_err(|error| format!("source catalog JSON is invalid: {error}"))?;
    applications
        .into_iter()
        .find(|application| {
            application.application_id == application_id
                && application.package_name == package_name
                && application.architecture == architecture
                && (!require_removable || application.removable)
        })
        .ok_or_else(|| "source catalog does not authorize this application".to_owned())
}

fn verify_ed25519(message: &[u8], signature_hex: &str) -> Result<(), String> {
    verify_ed25519_with(message, signature_hex, FEED_PUBLIC_KEY_HEX)
}

fn verify_ed25519_with(message: &[u8], signature_hex: &str, public_key_hex: &str) -> Result<(), String> {
    let key_bytes = decode_hex_32(public_key_hex)?;
    let signature_bytes = decode_hex_64(signature_hex)?;
    let public_key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key_bytes);
    public_key
        .verify(message, &signature_bytes)
        .map_err(|_| "signature verification failed".to_owned())
}

fn decode_hex_32(input: &str) -> Result<[u8; 32], String> {
    let bytes = decode_hex(input)?;
    let mut out = [0_u8; 32];
    if bytes.len() != out.len() {
        return Err("feed public key has an invalid length".to_owned());
    }
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_hex_64(input: &str) -> Result<[u8; 64], String> {
    let bytes = decode_hex(input)?;
    let mut out = [0_u8; 64];
    if bytes.len() != out.len() {
        return Err("catalog signature has an invalid length".to_owned());
    }
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if input.len() % 2 != 0 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("catalog signature is not valid hex".to_owned());
    }
    (0..input.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&input[index..index + 2], 16)
                .map_err(|_| "catalog signature is not valid hex".to_owned())
        })
        .collect()
}

fn validate_official_apt_record(
    plan: &OperationPlan,
    application: &OfficialAptApplication,
) -> Result<(), String> {
    let policy = command_output(APT_CACHE_BIN, &["policy", application.package_name.as_str()])?;
    if !policy_version_has_repository(
        &policy,
        &plan.payload.target_version,
        application.repository_url.as_str(),
    ) {
        return Err("target version is not backed by the allowed official repository".to_owned());
    }

    let selector = format!(
        "{}={}",
        application.package_name, plan.payload.target_version
    );
    let output = command_output(APT_CACHE_BIN, &["show", &selector])?;
    let fields = parse_control_fields(&output);
    if fields.get("Package").map(String::as_str) != Some(application.package_name.as_str())
        || fields.get("Version") != Some(&plan.payload.target_version)
        || fields.get("Architecture").map(String::as_str) != Some(application.architecture.as_str())
        || fields.get("SHA256") != Some(&plan.payload.sha256)
        || fields.get("Size").and_then(|value| value.parse().ok()) != Some(plan.payload.size)
    {
        return Err("operation plan does not match the system's verified APT index".to_owned());
    }
    Ok(())
}

fn policy_version_has_repository(policy: &str, version: &str, repository_url: &str) -> bool {
    let mut in_target_version = false;
    for line in policy.lines() {
        let trimmed = line.trim();
        let fields: Vec<_> = trimmed.split_whitespace().collect();
        let version_line = match fields.as_slice() {
            ["***", found, priority] if priority.bytes().all(|byte| byte.is_ascii_digit()) => {
                Some(*found)
            }
            [found, priority] if priority.bytes().all(|byte| byte.is_ascii_digit()) => Some(*found),
            _ => None,
        };
        if let Some(found) = version_line {
            in_target_version = found == version;
            continue;
        }
        if in_target_version
            && fields.iter().any(|field| {
                field
                    .trim_end_matches('/')
                    .eq_ignore_ascii_case(repository_url)
            })
        {
            return true;
        }
    }
    false
}

fn stage_verified_deb(
    plan: &OperationPlan,
    cache_root: &Path,
    caller_uid: u32,
) -> Result<PathBuf, String> {
    let cache_subdirectory = match plan.payload.action {
        OperationAction::InstallVerifiedDeb
        | OperationAction::InstallVerifiedWebsiteDeb
        | OperationAction::InstallSelfUpdate => "downloads",
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
    let mut command = clean_command(DPKG_BIN);
    command.arg("--install").arg(staged_path);
    execute_streaming(command, "dpkg install")
}

fn fixed_remove_command(package_name: &str) -> PlannedCommand {
    PlannedCommand {
        executable: DPKG_BIN,
        arguments: vec!["--remove".to_owned(), package_name.to_owned()],
    }
}

fn execute_fixed_remove(package_name: &str) -> Result<(), String> {
    let mut command = clean_command(DPKG_BIN);
    command.arg("--remove").arg(package_name);
    execute_streaming(command, "dpkg remove")
}

fn execute_streaming(mut command: Command, label: &str) -> Result<(), String> {
    emit_progress("phase", "system", &format!("开始执行 {label}"));
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot execute {label}: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("cannot capture {label} stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("cannot capture {label} stderr"))?;
    let budget = Arc::new(AtomicUsize::new(0));
    let truncated = Arc::new(AtomicBool::new(false));
    let error_tail = Arc::new(Mutex::new(VecDeque::<String>::with_capacity(12)));

    let stdout_thread = {
        let budget = Arc::clone(&budget);
        let truncated = Arc::clone(&truncated);
        std::thread::spawn(move || forward_output(stdout, "stdout", budget, truncated, None))
    };
    let stderr_thread = {
        let budget = Arc::clone(&budget);
        let truncated = Arc::clone(&truncated);
        let error_tail = Arc::clone(&error_tail);
        std::thread::spawn(move || {
            forward_output(stderr, "stderr", budget, truncated, Some(error_tail))
        })
    };
    let status = child
        .wait()
        .map_err(|error| format!("cannot wait for {label}: {error}"))?;
    stdout_thread
        .join()
        .map_err(|_| format!("{label} stdout reader failed"))??;
    stderr_thread
        .join()
        .map_err(|_| format!("{label} stderr reader failed"))??;
    if !status.success() {
        let rendered = error_tail
            .lock()
            .map_err(|_| format!("cannot read {label} error output"))?
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(hint) = dependency_failure_hint(&rendered) {
            return Err(format!("{label} failed: {hint}"));
        }
        return Err(format!("{label} failed: {rendered}"));
    }
    Ok(())
}

/// When `dpkg --install` fails because the package declares dependencies that are
/// not satisfied on this system, dpkg leaves the package unconfigured and prints
/// a compact error. UManager deliberately does not resolve dependencies itself,
/// so instead of echoing the raw dpkg tail back, surface a clear, actionable
/// message that names the missing packages and the standard `apt-get -f` remedy.
/// Returns `None` for any other failure so callers keep the original error text.
fn dependency_failure_hint(rendered: &str) -> Option<String> {
    if !rendered.to_ascii_lowercase().contains("dependency problems") {
        return None;
    }
    let missing = extract_missing_packages(rendered);
    let mut message = "安装包存在未满足的系统依赖，dpkg 未能完成配置；请先在终端执行 \
sudo apt-get install -f 补装依赖后重试"
        .to_owned();
    if !missing.is_empty() {
        message.push_str("。缺少的依赖：");
        message.push_str(&missing.join("、"));
    }
    Some(message)
}

/// Extracts the concrete package names dpkg reports as missing from a dependency
/// failure tail. Handles both the `Package X is not installed.` line and the
/// `... depends on X ...; however:` line (dropping any version constraint).
fn extract_missing_packages(rendered: &str) -> Vec<String> {
    let mut packages: Vec<String> = Vec::new();
    for raw_line in rendered.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("Package ") {
            if let Some(name) = rest.strip_suffix(" is not installed.") {
                push_unique_package(&mut packages, name.trim());
            }
        }
        if let Some(position) = line.find(" depends on ") {
            let rest = &line[position + " depends on ".len()..];
            let name = rest
                .split(|character: char| character == ';' || character == '(' || character.is_whitespace())
                .next()
                .unwrap_or("")
                .trim();
            push_unique_package(&mut packages, name);
        }
    }
    packages
}

fn push_unique_package(packages: &mut Vec<String>, name: &str) {
    if valid_package_name(name) && !packages.iter().any(|existing| existing == name) {
        packages.push(name.to_owned());
    }
}

fn forward_output<R: Read>(
    reader: R,
    stream: &'static str,
    budget: Arc<AtomicUsize>,
    truncated: Arc<AtomicBool>,
    error_tail: Option<Arc<Mutex<VecDeque<String>>>>,
) -> Result<(), String> {
    for line in BufReader::new(reader).split(b'\n') {
        let line = line.map_err(|error| format!("cannot read dpkg {stream}: {error}"))?;
        let sanitized = sanitize_log_line(&String::from_utf8_lossy(&line));
        if sanitized.is_empty() {
            continue;
        }
        if let Some(tail) = &error_tail {
            let mut tail = tail
                .lock()
                .map_err(|_| "cannot retain dpkg error output".to_owned())?;
            if tail.len() == 12 {
                tail.pop_front();
            }
            tail.push_back(sanitized.clone());
        }
        let previous = budget.fetch_add(sanitized.len(), Ordering::Relaxed);
        if previous >= MAX_LOG_BYTES {
            if !truncated.swap(true, Ordering::Relaxed) {
                emit_progress("warning", "system", "详细日志超过 256 KiB，后续输出已截断");
            }
            continue;
        }
        emit_progress("log", stream, &sanitized);
    }
    Ok(())
}

fn emit_progress(kind: &str, stream: &str, message: &str) {
    let event = HelperProgressEvent {
        kind,
        stream,
        message,
    };
    if let Ok(encoded) = serde_json::to_string(&event) {
        eprintln!("{LOG_EVENT_PREFIX}{encoded}");
    }
}

fn sanitize_log_line(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len().min(MAX_LOG_LINE_CHARS));
    let mut index = 0;
    while index < bytes.len() && output.chars().count() < MAX_LOG_LINE_CHARS {
        if bytes[index] == 0x1b {
            index += 1;
            if index < bytes.len() && bytes[index] == b'[' {
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            } else if index < bytes.len() && bytes[index] == b']' {
                index += 1;
                while index < bytes.len() && bytes[index] != 0x07 {
                    index += 1;
                }
                index = (index + 1).min(bytes.len());
            } else {
                index = (index + 1).min(bytes.len());
            }
            continue;
        }
        let rest = &input[index..];
        let Some(character) = rest.chars().next() else {
            break;
        };
        index += character.len_utf8();
        if character == '\t' {
            output.push_str("    ");
        } else if !character.is_control() {
            output.push(character);
        }
    }
    output
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
        .env("PATH", SAFE_SYSTEM_PATH)
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
        assert_eq!(
            invocation.expected_action,
            HelperAction::Install(OperationAction::InstallLocalDeb)
        );
        assert_eq!(invocation.mode, ExecutionMode::Execute);

        let removal = [
            "remove-managed-package",
            "--plan",
            "/tmp/plan.json",
            "--dry-run",
        ]
        .map(OsString::from);
        let invocation = parse_invocation(removal).unwrap();
        assert_eq!(
            invocation.expected_action,
            HelperAction::Remove(RemovalAction::RemoveManagedPackage)
        );
        assert_eq!(invocation.mode, ExecutionMode::DryRun);

        let self_removal =
            ["remove-umanager", "--plan", "/tmp/plan.json", "--execute"].map(OsString::from);
        let invocation = parse_invocation(self_removal).unwrap();
        assert_eq!(
            invocation.expected_action,
            HelperAction::Remove(RemovalAction::RemoveUmanager)
        );
        assert_eq!(invocation.mode, ExecutionMode::Execute);
    }

    #[test]
    fn validates_only_allowlisted_official_apt_actions() {
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
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
        };
        let mut plan = OperationPlan::new(payload).unwrap();
        let catalog = load_catalog().unwrap();
        assert!(validate_official_apt_constraints(&plan, &catalog).is_ok());
        plan.payload.architecture = "arm64".to_owned();
        assert!(validate_official_apt_constraints(&plan, &catalog).is_err());
        plan.payload.architecture = "amd64".to_owned();
        plan.payload.package_name = "google-chrome-stable".to_owned();
        assert!(validate_official_apt_constraints(&plan, &catalog).is_err());
    }

    #[test]
    fn website_update_shape_is_locked_to_the_wechat_and_flclash_allowlist() {
        let payload = umanager_plan::PlanPayload {
            schema_version: umanager_plan::PLAN_SCHEMA_VERSION,
            action: OperationAction::InstallVerifiedWebsiteDeb,
            application_id: "wechat".to_owned(),
            package_name: "wechat".to_owned(),
            installed_version: Some("4.1.1.8".to_owned()),
            target_version: "4.1.2.1".to_owned(),
            architecture: "amd64".to_owned(),
            deb_path: "/cache/downloads/wechat.deb".to_owned(),
            sha256: "a".repeat(64),
            size: 1,
            created_at_unix_seconds: 1,
            expires_at_unix_seconds: 2,
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
        };
        let mut plan = OperationPlan::new(payload).unwrap();
        let catalog = load_catalog().unwrap();
        assert_eq!(
            website_application_for_plan(&plan, &catalog).unwrap().package_name,
            "wechat"
        );
        plan.payload.application_id = "flclash".to_owned();
        plan.payload.package_name = "flclash".to_owned();
        assert_eq!(
            website_application_for_plan(&plan, &catalog).unwrap().package_name,
            "flclash"
        );
        plan.payload.package_name = "bash".to_owned();
        assert!(website_application_for_plan(&plan, &catalog).is_err());
        plan.payload.package_name = "flclash".to_owned();
        plan.payload.application_id = "flclash".to_owned();
        plan.payload.installed_version = None;
        assert!(website_application_for_plan(&plan, &catalog).is_ok());
        plan.payload.architecture = "arm64".to_owned();
        assert!(website_application_for_plan(&plan, &catalog).is_err());
    }

    #[test]
    fn feed_added_authorization_rejects_missing_or_invalid_signatures() {
        // No catalog at all.
        assert!(feed_added_application(None, None, "x", "y", "amd64", false).is_err());
        // Invalid signature (never reaches the JSON parse, so the content is irrelevant).
        let json = r#"[{"applicationId":"x","packageName":"y","architecture":"amd64","removable":true,"source":{"kind":"browserImport","homepageUrl":"https://example.com"}}]"#;
        assert!(
            feed_added_application(
                Some(json),
                Some(&"0".repeat(128)),
                "x",
                "y",
                "amd64",
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn source_chain_authorizes_only_a_fully_verified_application() {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let secret = |seed: u8| {
            let kp = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
            (hex(kp.public_key().as_ref()), kp)
        };
        let (_, source_kp) = secret(7);
        let (central_pub, central_kp) = secret(9);

        let json = r#"[{"applicationId":"qq","packageName":"linuxqq","displayName":"QQ","vendor":"Tencent","architecture":"amd64","removable":true,"source":{"kind":"browserImport","homepageUrl":"https://example.com"}}]"#;
        let source_ref = umanager_plan::SourceRef {
            source_id: "tencent".to_owned(),
            feed_url: "https://example.com/feed.tencent.json".to_owned(),
            public_key_hex: hex(source_kp.public_key().as_ref()),
        };
        let canonical = serde_json::to_vec(&source_ref).unwrap();
        let endorsement = hex(central_kp.sign(&canonical).as_ref());
        let source_sig = hex(source_kp.sign(json.as_bytes()).as_ref());

        // Happy path: central endorses source_ref, source key signs its catalog.
        assert!(
            feed_added_application_source(
                &source_ref, Some(&endorsement), Some(json), Some(&source_sig),
                "qq", "linuxqq", "amd64", true, &central_pub,
            )
            .is_ok()
        );

        // Bad central endorsement (signed by the wrong key) is rejected.
        let wrong_endorsement = hex(source_kp.sign(&canonical).as_ref());
        assert!(
            feed_added_application_source(
                &source_ref, Some(&wrong_endorsement), Some(json), Some(&source_sig),
                "qq", "linuxqq", "amd64", true, &central_pub,
            )
            .is_err()
        );

        // Bad source catalog signature is rejected before the lookup even matters.
        assert!(
            feed_added_application_source(
                &source_ref, Some(&endorsement), Some(json), Some(&hex(source_kp.sign(b"other").as_ref())),
                "qq", "linuxqq", "amd64", true, &central_pub,
            )
            .is_err()
        );

        // A valid chain but the app is absent from the source catalog is rejected.
        assert!(
            feed_added_application_source(
                &source_ref, Some(&endorsement), Some(json), Some(&source_sig),
                "not-in-catalog", "linuxqq", "amd64", true, &central_pub,
            )
            .is_err()
        );

        // Incomplete chain (missing source signature) is rejected.
        assert!(
            feed_added_application_source(
                &source_ref, Some(&endorsement), Some(json), None,
                "qq", "linuxqq", "amd64", true, &central_pub,
            )
            .is_err()
        );
    }

    #[test]
    fn distinguishes_a_new_install_from_an_installed_package() {
        assert_eq!(
            parse_official_installed_state("ii \t151.0.1-1\tamd64").unwrap(),
            OfficialInstalledState::Installed {
                version: "151.0.1-1".to_owned(),
                architecture: "amd64".to_owned(),
            }
        );
        assert_eq!(
            parse_official_installed_state("rc \t151.0.1-1\tamd64").unwrap(),
            OfficialInstalledState::NotInstalled
        );
        assert!(parse_official_installed_state("ii \tbroken").is_err());
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
    fn removal_exposes_only_the_fixed_dpkg_remove_shape() {
        let command = fixed_remove_command("wechat");
        assert_eq!(command.executable, "/usr/bin/dpkg");
        assert_eq!(command.arguments, ["--remove", "wechat"]);
    }

    #[test]
    fn validates_only_allowlisted_removal_actions() {
        let payload = umanager_plan::RemovalPlanPayload {
            schema_version: umanager_plan::PLAN_SCHEMA_VERSION,
            action: RemovalAction::RemoveManagedPackage,
            application_id: "vscode".to_owned(),
            package_name: "code".to_owned(),
            installed_version: "1.0".to_owned(),
            architecture: "amd64".to_owned(),
            created_at_unix_seconds: 1,
            expires_at_unix_seconds: 2,
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
        };
        let mut plan = RemovalPlan::new(payload).unwrap();
        let catalog = load_catalog().unwrap();
        assert!(validate_removal_constraints(&plan, &catalog).is_ok());
        let application = validate_removal_constraints(&plan, &catalog).unwrap();
        assert!(installed_state_matches_removal_plan(
            "ii \t1.0\tamd64",
            &plan,
            &application
        ));
        assert!(!installed_state_matches_removal_plan(
            "ii \t2.0\tamd64",
            &plan,
            &application
        ));
        assert!(!installed_state_matches_removal_plan(
            "rc \t1.0\tamd64",
            &plan,
            &application
        ));
        plan.payload.package_name = "bash".to_owned();
        assert!(validate_removal_constraints(&plan, &catalog).is_err());
    }

    #[test]
    fn self_removal_targets_only_the_exact_umanager_package() {
        let payload = umanager_plan::RemovalPlanPayload {
            schema_version: umanager_plan::PLAN_SCHEMA_VERSION,
            action: RemovalAction::RemoveUmanager,
            application_id: UMANAGER_APPLICATION_ID.to_owned(),
            package_name: UMANAGER_PACKAGE_NAME.to_owned(),
            installed_version: "0.1.0".to_owned(),
            architecture: "amd64".to_owned(),
            created_at_unix_seconds: 1,
            expires_at_unix_seconds: 2,
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
        };
        let mut plan = RemovalPlan::new(payload).unwrap();
        let catalog = load_catalog().unwrap();
        let application = validate_removal_constraints(&plan, &catalog).unwrap();
        assert_eq!(application.package_name, UMANAGER_PACKAGE_NAME);
        assert!(installed_state_matches_removal_plan(
            "ii \t0.1.0\tamd64",
            &plan,
            &application
        ));
        plan.payload.package_name = "dpkg".to_owned();
        assert!(validate_removal_constraints(&plan, &catalog).is_err());
    }

    #[test]
    fn self_update_plan_is_locked_to_the_configured_umanager_source() {
        let catalog = load_catalog().unwrap();
        let source = catalog.self_update_source().unwrap();
        let payload = umanager_plan::PlanPayload {
            schema_version: umanager_plan::PLAN_SCHEMA_VERSION,
            action: OperationAction::InstallSelfUpdate,
            application_id: source.application_id.clone(),
            package_name: source.package_name.clone(),
            installed_version: Some("0.1.0".to_owned()),
            target_version: "0.1.1".to_owned(),
            architecture: source.architecture.clone(),
            deb_path: "/cache/downloads/u-manager.deb".to_owned(),
            sha256: "a".repeat(64),
            size: 1,
            created_at_unix_seconds: 1,
            expires_at_unix_seconds: 2,
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
        };
        let mut plan = OperationPlan::new(payload).unwrap();
        assert!(self_update_plan_matches(&plan, source));
        plan.payload.package_name = "flclash".to_owned();
        assert!(!self_update_plan_matches(&plan, source));
        plan.payload.package_name = source.package_name.clone();
        plan.payload.architecture = "arm64".to_owned();
        assert!(!self_update_plan_matches(&plan, source));
        plan.payload.architecture = source.architecture.clone();
        plan.payload.action = OperationAction::InstallVerifiedWebsiteDeb;
        assert!(!self_update_plan_matches(&plan, source));
    }

    #[test]
    fn privileged_commands_receive_only_a_fixed_system_path_and_locale() {
        let command = clean_command(DPKG_BIN);
        let environment: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|item| item.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(
            environment.get("PATH").and_then(Option::as_deref),
            Some(SAFE_SYSTEM_PATH)
        );
        assert_eq!(
            environment.get("LC_ALL").and_then(Option::as_deref),
            Some("C")
        );
        assert_eq!(environment.len(), 4);
    }

    #[test]
    fn sanitizes_terminal_control_sequences_and_limits_lines() {
        assert_eq!(
            sanitize_log_line("normal\x1b[31m red\x1b[0m\ttext\0"),
            "normal red    text"
        );
        assert_eq!(
            sanitize_log_line("title\x1b]0;spoofed\x07safe"),
            "titlesafe"
        );
        assert_eq!(
            sanitize_log_line(&"a".repeat(MAX_LOG_LINE_CHARS + 20)).len(),
            MAX_LOG_LINE_CHARS
        );
    }

    #[test]
    fn surfaces_actionable_hint_for_unmet_dependencies() {
        let tail = "dpkg: dependency problems prevent configuration of bytedance-feishu-stable:\n\
                    bytedance-feishu-stable depends on pulseaudio-utils; however:\n\
                     Package pulseaudio-utils is not installed.\n\
                    dpkg: error processing package bytedance-feishu-stable (--install):\n\
                     dependency problems - leaving unconfigured\n\
                    Errors were encountered while processing:\n\
                     bytedance-feishu-stable";
        let hint = dependency_failure_hint(tail).unwrap();
        assert!(hint.contains("sudo apt-get install -f"));
        assert!(hint.contains("pulseaudio-utils"));
    }

    #[test]
    fn non_dependency_failures_keep_the_original_error() {
        assert!(dependency_failure_hint("dpkg: error: cannot access archive").is_none());
        assert!(dependency_failure_hint("dpkg remove failed for unrelated reasons").is_none());
    }

    #[test]
    fn extracts_versioned_and_multiple_missing_packages() {
        let tail = "x depends on libgtk-3-0 (>= 3.24); however:\n\
                     Package libgtk-3-0 is not installed.\n\
                    x depends on pulseaudio-utils; however:\n\
                     Package pulseaudio-utils is not installed.\n";
        assert_eq!(
            extract_missing_packages(tail),
            vec!["libgtk-3-0".to_owned(), "pulseaudio-utils".to_owned()]
        );
    }

    #[test]
    fn validates_debian_package_name_shape() {
        assert!(valid_package_name("example-app+edition.1"));
        assert!(!valid_package_name("-option"));
        assert!(!valid_package_name("Example"));
        assert!(!valid_package_name("name/other"));
    }

    #[test]
    fn ties_the_target_version_to_the_exact_repository_block() {
        let policy = r#"Package:
  Installed: 1
  Candidate: 2
  Version table:
     2 500
        500 https://dl.google.com/linux/chrome-stable/deb stable/main amd64 Packages
 *** 1 100
        100 /var/lib/dpkg/status
"#;
        assert!(policy_version_has_repository(
            policy,
            "2",
            "https://dl.google.com/linux/chrome-stable/deb"
        ));
        assert!(!policy_version_has_repository(
            policy,
            "2",
            "https://dl.google.com.evil.example/linux/chrome-stable/deb"
        ));
        assert!(!policy_version_has_repository(
            policy,
            "1",
            "https://dl.google.com/linux/chrome-stable/deb"
        ));
    }
}
