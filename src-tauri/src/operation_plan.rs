use crate::installation;
use crate::scanner;
use crate::source_engine;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use umanager_catalog::{Application, Catalog};
use umanager_plan::{
    MAX_PLAN_LIFETIME_SECONDS, OperationAction, OperationPlan, PLAN_SCHEMA_VERSION, PlanPayload,
    RemovalAction, RemovalPlan, RemovalPlanPayload,
};

const DPKG_BIN: &str = "/usr/bin/dpkg";
const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArtifact {
    pub(crate) plan: OperationPlan,
    pub(crate) plan_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlanArtifact {
    pub(crate) plan: RemovalPlan,
    pub(crate) plan_path: String,
}

const LOG_EVENT_PREFIX: &str = "UMANAGER_EVENT\t";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgressEvent {
    pub plan_id: String,
    pub kind: String,
    pub stream: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperProgressEvent {
    kind: String,
    stream: String,
    message: String,
}

pub type ProgressCallback = Arc<dyn Fn(OperationProgressEvent) + Send + Sync>;

pub async fn create_install_plan(
    catalog: &Catalog,
    app: &Application,
    cache_dir: PathBuf,
) -> Result<PlanArtifact, String> {
    let verified = source_engine::verify_cached(app, &cache_dir).await?;
    let package_name = app.package_name.clone();
    let catalog_for_scan = catalog.clone();
    let installed_version = tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        let scan = scanner::scan(&catalog_for_scan)?;
        Ok(scan
            .packages
            .into_iter()
            .find(|item| item.package_name == package_name)
            .map(|item| item.installed_version))
    })
    .await
    .map_err(|error| format!("计划本机状态任务异常结束：{error}"))??;
    ensure_installable_transition(app, installed_version.as_deref(), &verified.plan.version)?;

    // Under the central metadata feed model every managed application is downloaded
    // from a pinned official URL and installed with a fixed `dpkg --install`, so the
    // website (feed-verified) helper action is used for all of them.
    let action = OperationAction::InstallVerifiedWebsiteDeb;
    let (catalog_json, catalog_signature) = signed_catalog_auth(app)?;
    let created = unix_timestamp();
    let plan = OperationPlan::new(PlanPayload {
        schema_version: PLAN_SCHEMA_VERSION,
        action,
        application_id: app.application_id.clone(),
        package_name: verified.plan.package_name,
        installed_version,
        target_version: verified.plan.version,
        architecture: verified.plan.architecture,
        deb_path: verified.plan.target_path,
        sha256: verified.actual_sha256,
        size: verified.actual_size,
        created_at_unix_seconds: created,
        expires_at_unix_seconds: created + MAX_PLAN_LIFETIME_SECONDS,
        catalog_json,
        catalog_signature,
    })?;
    let path = persist_immutable_plan(&cache_dir.join("plans"), &plan)?;
    Ok(PlanArtifact {
        plan,
        plan_path: path.to_string_lossy().into_owned(),
    })
}

/// Returns the signed catalog auth pair when the application was added by the
/// metadata feed (rather than compiled into the embedded catalog).
fn signed_catalog_auth(app: &Application) -> Result<(Option<String>, Option<String>), String> {
    let embedded = Catalog::load()?;
    if embedded.by_application_id(&app.application_id).is_some() {
        return Ok((None, None));
    }
    crate::feed::catalog_auth()
        .map(|(json, signature)| (Some(json), Some(signature)))
        .ok_or_else(|| "无法取得该应用已签名的软件源目录".to_owned())
}

fn ensure_installable_transition(
    app: &Application,
    installed_version: Option<&str>,
    target_version: &str,
) -> Result<(), String> {
    let Some(installed) = installed_version else {
        return Ok(());
    };
    if !version_is_newer(installed, target_version) {
        return Err(format!(
            "{} 当前已是最新版本，拒绝生成重装或降级计划",
            app.display_name
        ));
    }
    Ok(())
}

pub fn create_removal_plan(
    catalog: &Catalog,
    cache_dir: &Path,
    package_name: &str,
) -> Result<RemovalPlanArtifact, String> {
    let application = catalog
        .by_package_name(package_name)
        .filter(|item| item.removable)
        .ok_or_else(|| "该软件包不在 UManager 卸载白名单中".to_owned())?;
    let scan = scanner::scan(catalog)?;
    let package = scan
        .packages
        .iter()
        .find(|item| item.package_name == package_name)
        .ok_or_else(|| "软件包未安装或已不再由 UManager 管理".to_owned())?;
    let (catalog_json, catalog_signature) = signed_catalog_auth(application)?;
    let created = unix_timestamp();
    let plan = RemovalPlan::new(RemovalPlanPayload {
        schema_version: PLAN_SCHEMA_VERSION,
        action: RemovalAction::RemoveManagedPackage,
        application_id: application.application_id.clone(),
        package_name: package.package_name.clone(),
        installed_version: package.installed_version.clone(),
        architecture: package.architecture.clone(),
        created_at_unix_seconds: created,
        expires_at_unix_seconds: created + MAX_PLAN_LIFETIME_SECONDS,
        catalog_json,
        catalog_signature,
    })?;
    let plan_path = persist_immutable_removal_plan(&cache_dir.join("plans"), &plan)?;
    Ok(RemovalPlanArtifact {
        plan,
        plan_path: plan_path.to_string_lossy().into_owned(),
    })
}

pub fn create_self_removal_plan(cache_dir: &Path) -> Result<RemovalPlanArtifact, String> {
    let info = installation::detect()?;
    if !info.can_self_remove {
        return Err("当前运行的 UManager 不是由 Debian 包安装，无法通过包管理器卸载".to_owned());
    }
    let installed_version = info
        .package_version
        .ok_or_else(|| "无法确定已安装的 UManager 包版本".to_owned())?;
    let architecture = info
        .architecture
        .ok_or_else(|| "无法确定已安装的 UManager 包架构".to_owned())?;
    let created = unix_timestamp();
    let plan = RemovalPlan::new(RemovalPlanPayload {
        schema_version: PLAN_SCHEMA_VERSION,
        action: RemovalAction::RemoveUmanager,
        application_id: installation::APPLICATION_ID.to_owned(),
        package_name: installation::PACKAGE_NAME.to_owned(),
        installed_version,
        architecture,
        created_at_unix_seconds: created,
        expires_at_unix_seconds: created + MAX_PLAN_LIFETIME_SECONDS,
        catalog_json: None,
        catalog_signature: None,
    })?;
    let plan_path = persist_immutable_removal_plan(&cache_dir.join("plans"), &plan)?;
    Ok(RemovalPlanArtifact {
        plan,
        plan_path: plan_path.to_string_lossy().into_owned(),
    })
}

pub async fn create_self_update_plan(cache_dir: &Path) -> Result<PlanArtifact, String> {
    let catalog = Catalog::load()?;
    let source = catalog
        .self_update_source()
        .ok_or_else(|| "软件源未配置 UManager 自更新".to_owned())?;
    let application = source.to_application();
    let info = installation::detect()?;
    if !info.can_self_remove {
        return Err("当前运行的 UManager 不是由 Debian 包安装，无法自更新".to_owned());
    }
    let installed_version = info
        .package_version
        .ok_or_else(|| "无法确定已安装的 UManager 包版本".to_owned())?;
    let verified = source_engine::verify_cached(&application, cache_dir).await?;
    if !version_is_newer(&installed_version, &verified.plan.version) {
        return Err("UManager 已是最新版本，拒绝生成重装或降级计划".to_owned());
    }
    let created = unix_timestamp();
    let plan = OperationPlan::new(PlanPayload {
        schema_version: PLAN_SCHEMA_VERSION,
        action: OperationAction::InstallSelfUpdate,
        application_id: application.application_id.clone(),
        package_name: verified.plan.package_name,
        installed_version: Some(installed_version),
        target_version: verified.plan.version,
        architecture: verified.plan.architecture,
        deb_path: verified.plan.target_path,
        sha256: verified.actual_sha256,
        size: verified.actual_size,
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

pub(crate) fn persist_immutable_plan(
    directory: &Path,
    plan: &OperationPlan,
) -> Result<PathBuf, String> {
    fs::create_dir_all(directory).map_err(|error| format!("无法创建操作计划目录：{error}"))?;
    let path = directory.join(format!("{}.json", plan.plan_id));
    let encoded =
        serde_json::to_vec_pretty(plan).map_err(|error| format!("无法序列化操作计划：{error}"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("无法以不可覆盖方式创建操作计划：{error}"))?;
    file.write_all(&encoded)
        .map_err(|error| format!("无法写入操作计划：{error}"))?;
    file.sync_all()
        .map_err(|error| format!("无法同步操作计划：{error}"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
        .map_err(|error| format!("无法将操作计划设为只读：{error}"))?;
    Ok(path)
}

fn persist_immutable_removal_plan(directory: &Path, plan: &RemovalPlan) -> Result<PathBuf, String> {
    fs::create_dir_all(directory).map_err(|error| format!("无法创建操作计划目录：{error}"))?;
    let path = directory.join(format!("{}.json", plan.plan_id));
    let encoded =
        serde_json::to_vec_pretty(plan).map_err(|error| format!("无法序列化卸载计划：{error}"))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| format!("无法以不可覆盖方式创建卸载计划：{error}"))?;
    file.write_all(&encoded)
        .map_err(|error| format!("无法写入卸载计划：{error}"))?;
    file.sync_all()
        .map_err(|error| format!("无法同步卸载计划：{error}"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))
        .map_err(|error| format!("无法将卸载计划设为只读：{error}"))?;
    Ok(path)
}

pub fn run_install_dry_run(cache_dir: &Path, plan_id: &str) -> Result<serde_json::Value, String> {
    let (action, helper_action) = resolve_install_action(cache_dir, plan_id)?;
    run_helper(cache_dir, plan_id, action, helper_action, "--dry-run", None)
}

pub fn execute_install(
    cache_dir: &Path,
    plan_id: &str,
    progress: ProgressCallback,
) -> Result<serde_json::Value, String> {
    let (action, helper_action) = resolve_install_action(cache_dir, plan_id)?;
    run_helper(cache_dir, plan_id, action, helper_action, "--execute", Some(progress))
}

fn resolve_install_action(
    cache_dir: &Path,
    plan_id: &str,
) -> Result<(OperationAction, &'static str), String> {
    validate_plan_id(plan_id)?;
    let plan_path = cache_dir.join("plans").join(format!("{plan_id}.json"));
    let plan: OperationPlan = serde_json::from_slice(
        &fs::read(&plan_path).map_err(|error| format!("无法读取操作计划：{error}"))?,
    )
    .map_err(|error| format!("操作计划格式无效：{error}"))?;
    plan.verify_integrity()?;
    match plan.payload.action {
        OperationAction::InstallVerifiedDeb => {
            Ok((OperationAction::InstallVerifiedDeb, "install-verified-deb"))
        }
        OperationAction::InstallVerifiedWebsiteDeb => Ok((
            OperationAction::InstallVerifiedWebsiteDeb,
            "install-verified-website-deb",
        )),
        OperationAction::InstallLocalDeb => {
            Err("本地安装包请使用本地安装命令".to_owned())
        }
        OperationAction::InstallSelfUpdate => {
            Err("UManager 自更新请使用自更新命令".to_owned())
        }
    }
}

pub fn run_removal_dry_run(cache_dir: &Path, plan_id: &str) -> Result<serde_json::Value, String> {
    run_removal_helper(
        cache_dir,
        plan_id,
        RemovalAction::RemoveManagedPackage,
        "remove-managed-package",
        "--dry-run",
        None,
    )
}

pub fn execute_removal(
    cache_dir: &Path,
    plan_id: &str,
    progress: ProgressCallback,
) -> Result<serde_json::Value, String> {
    run_removal_helper(
        cache_dir,
        plan_id,
        RemovalAction::RemoveManagedPackage,
        "remove-managed-package",
        "--execute",
        Some(progress),
    )
}

pub fn run_self_removal_dry_run(
    cache_dir: &Path,
    plan_id: &str,
) -> Result<serde_json::Value, String> {
    run_removal_helper(
        cache_dir,
        plan_id,
        RemovalAction::RemoveUmanager,
        "remove-umanager",
        "--dry-run",
        None,
    )
}

pub fn execute_self_removal(
    cache_dir: &Path,
    plan_id: &str,
    progress: ProgressCallback,
) -> Result<serde_json::Value, String> {
    run_removal_helper(
        cache_dir,
        plan_id,
        RemovalAction::RemoveUmanager,
        "remove-umanager",
        "--execute",
        Some(progress),
    )
}

pub fn run_self_update_dry_run(cache_dir: &Path, plan_id: &str) -> Result<serde_json::Value, String> {
    run_helper(
        cache_dir,
        plan_id,
        OperationAction::InstallSelfUpdate,
        "install-umanager",
        "--dry-run",
        None,
    )
}

pub fn execute_self_update(
    cache_dir: &Path,
    plan_id: &str,
    progress: ProgressCallback,
) -> Result<serde_json::Value, String> {
    run_helper(
        cache_dir,
        plan_id,
        OperationAction::InstallSelfUpdate,
        "install-umanager",
        "--execute",
        Some(progress),
    )
}

pub fn run_local_dry_run(cache_dir: &Path, plan_id: &str) -> Result<serde_json::Value, String> {
    run_helper(
        cache_dir,
        plan_id,
        OperationAction::InstallLocalDeb,
        "install-local-deb",
        "--dry-run",
        None,
    )
}

pub fn execute_local_install(
    cache_dir: &Path,
    plan_id: &str,
    progress: ProgressCallback,
) -> Result<serde_json::Value, String> {
    run_helper(
        cache_dir,
        plan_id,
        OperationAction::InstallLocalDeb,
        "install-local-deb",
        "--execute",
        Some(progress),
    )
}

fn run_helper(
    cache_dir: &Path,
    plan_id: &str,
    expected_action: OperationAction,
    helper_action: &str,
    mode: &str,
    progress: Option<ProgressCallback>,
) -> Result<serde_json::Value, String> {
    validate_plan_id(plan_id)?;
    let plan_path = cache_dir.join("plans").join(format!("{plan_id}.json"));
    let plan: OperationPlan = serde_json::from_slice(
        &fs::read(&plan_path).map_err(|error| format!("无法读取操作计划：{error}"))?,
    )
    .map_err(|error| format!("操作计划格式无效：{error}"))?;
    plan.verify_integrity()?;
    if plan.payload.action != expected_action {
        return Err("操作计划动作与请求不一致".to_owned());
    }
    let output = run_privileged_helper(&plan.plan_id, helper_action, &plan_path, mode, progress)?;
    serde_json::from_slice(&output).map_err(|error| format!("特权 helper 返回了无效结果：{error}"))
}

fn run_removal_helper(
    cache_dir: &Path,
    plan_id: &str,
    expected_action: RemovalAction,
    helper_action: &str,
    mode: &str,
    progress: Option<ProgressCallback>,
) -> Result<serde_json::Value, String> {
    validate_plan_id(plan_id)?;
    let plan_path = cache_dir.join("plans").join(format!("{plan_id}.json"));
    let plan: RemovalPlan = serde_json::from_slice(
        &fs::read(&plan_path).map_err(|error| format!("无法读取卸载计划：{error}"))?,
    )
    .map_err(|error| format!("卸载计划格式无效：{error}"))?;
    plan.verify_integrity()?;
    if plan.payload.action != expected_action {
        return Err("卸载计划动作与请求不一致".to_owned());
    }
    let output = run_privileged_helper(&plan.plan_id, helper_action, &plan_path, mode, progress)?;
    serde_json::from_slice(&output)
        .map_err(|error| format!("特权 helper 返回了无效卸载结果：{error}"))
}

fn run_privileged_helper(
    plan_id: &str,
    helper_action: &str,
    plan_path: &Path,
    mode: &str,
    progress: Option<ProgressCallback>,
) -> Result<Vec<u8>, String> {
    emit_progress(&progress, plan_id, "phase", "system", "等待系统授权");
    let mut child = std::process::Command::new("/usr/bin/pkexec")
        .args(["/usr/libexec/umanager-helper", helper_action, "--plan"])
        .arg(plan_path)
        .arg(mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动 Polkit 特权操作：{error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取特权 helper 输出".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取特权 helper 错误输出".to_owned())?;
    let mut errors = Vec::new();
    for line in BufReader::new(stderr).lines() {
        let line = line.map_err(|error| format!("无法读取特权 helper 日志：{error}"))?;
        if let Some(encoded) = line.strip_prefix(LOG_EVENT_PREFIX) {
            if let Ok(event) = serde_json::from_str::<HelperProgressEvent>(encoded) {
                emit_progress(
                    &progress,
                    plan_id,
                    &event.kind,
                    &event.stream,
                    &event.message,
                );
                continue;
            }
        }
        if !line.is_empty() {
            emit_progress(&progress, plan_id, "log", "stderr", &line);
            errors.push(line);
        }
    }
    let mut output = Vec::new();
    stdout
        .read_to_end(&mut output)
        .map_err(|error| format!("无法读取特权 helper 结果：{error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("无法等待特权 helper：{error}"))?;
    if !status.success() {
        return Err(format!("特权 helper 操作失败：{}", errors.join("\n")));
    }
    emit_progress(
        &progress,
        plan_id,
        "completed",
        "system",
        "系统包操作已成功完成",
    );
    Ok(output)
}

fn emit_progress(
    progress: &Option<ProgressCallback>,
    plan_id: &str,
    kind: &str,
    stream: &str,
    message: &str,
) {
    if let Some(callback) = progress {
        callback(OperationProgressEvent {
            plan_id: plan_id.to_owned(),
            kind: kind.to_owned(),
            stream: stream.to_owned(),
            message: message.to_owned(),
        });
    }
}

fn validate_plan_id(plan_id: &str) -> Result<(), String> {
    if plan_id.len() != 64 || !plan_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("操作计划 ID 无效".to_owned());
    }
    Ok(())
}

fn version_is_newer(installed: &str, candidate: &str) -> bool {
    clean_command(DPKG_BIN)
        .args(["--compare-versions", installed, "lt", candidate])
        .status()
        .is_ok_and(|status| status.success())
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

    #[test]
    fn rejects_untrusted_plan_identifiers_before_polkit() {
        assert!(run_install_dry_run(Path::new("/tmp/cache"), "../plan").is_err());
        assert!(run_install_dry_run(Path::new("/tmp/cache"), &"g".repeat(64)).is_err());
    }

    #[test]
    fn every_auto_installable_catalog_entry_has_a_removal_and_install_shape() {
        let catalog = Catalog::load().unwrap();
        for application in catalog.applications.iter().filter(|item| item.is_auto_installable()) {
            assert!(application.removable);
            assert!(catalog.by_application_id(&application.application_id).is_some());
        }
    }

    #[test]
    fn new_install_transition_does_not_require_a_previous_version() {
        let catalog = Catalog::load().unwrap();
        let code = catalog.by_package_name("code").unwrap();
        assert!(ensure_installable_transition(code, None, "2.0").is_ok());
    }
}
