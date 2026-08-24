use crate::adapters::vscode::download;
use crate::scanner::{self, UpdateState};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use umanager_plan::{
    MAX_PLAN_LIFETIME_SECONDS, OperationAction, OperationPlan, PLAN_SCHEMA_VERSION, PlanPayload,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanArtifact {
    pub(crate) plan: OperationPlan,
    pub(crate) plan_path: String,
}

pub async fn create_vscode_plan(cache_dir: PathBuf) -> Result<PlanArtifact, String> {
    let verified = download::verify_cached(cache_dir.clone()).await?;
    let scan = tauri::async_runtime::spawn_blocking(scanner::scan)
        .await
        .map_err(|error| format!("生成操作计划时扫描任务失败：{error}"))??;
    let package = scan
        .packages
        .iter()
        .find(|item| item.package_name == "code")
        .ok_or_else(|| "未检测到已安装的 Visual Studio Code".to_owned())?;
    if !matches!(package.update_state, UpdateState::UpdateAvailable) {
        return Err("VS Code 当前没有可安装的更高版本；拒绝生成重装或降级计划".to_owned());
    }
    if package.candidate_version.as_deref() != Some(&verified.plan.version) {
        return Err("APT 候选版本在下载后发生变化，请重新检查并下载".to_owned());
    }

    let created = unix_timestamp();
    let plan = OperationPlan::new(PlanPayload {
        schema_version: PLAN_SCHEMA_VERSION,
        action: OperationAction::InstallVerifiedDeb,
        application_id: "vscode".to_owned(),
        package_name: verified.plan.package_name,
        installed_version: Some(package.installed_version.clone()),
        target_version: verified.plan.version,
        architecture: verified.plan.architecture,
        deb_path: verified.plan.target_path,
        sha256: verified.actual_sha256,
        size: verified.actual_size,
        created_at_unix_seconds: created,
        expires_at_unix_seconds: created + MAX_PLAN_LIFETIME_SECONDS,
    })?;
    let plans_dir = cache_dir.join("plans");
    let plan_for_write = plan.clone();
    let plan_path = tauri::async_runtime::spawn_blocking(move || {
        persist_immutable_plan(&plans_dir, &plan_for_write)
    })
    .await
    .map_err(|error| format!("操作计划写入任务失败：{error}"))??;

    Ok(PlanArtifact {
        plan,
        plan_path: plan_path.to_string_lossy().into_owned(),
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

pub fn run_dry_run(cache_dir: &Path, plan_id: &str) -> Result<serde_json::Value, String> {
    run_helper(
        cache_dir,
        plan_id,
        OperationAction::InstallVerifiedDeb,
        "install-verified-deb",
        "--dry-run",
    )
}

pub fn run_local_dry_run(cache_dir: &Path, plan_id: &str) -> Result<serde_json::Value, String> {
    run_helper(
        cache_dir,
        plan_id,
        OperationAction::InstallLocalDeb,
        "install-local-deb",
        "--dry-run",
    )
}

pub fn execute_local_install(cache_dir: &Path, plan_id: &str) -> Result<serde_json::Value, String> {
    run_helper(
        cache_dir,
        plan_id,
        OperationAction::InstallLocalDeb,
        "install-local-deb",
        "--execute",
    )
}

fn run_helper(
    cache_dir: &Path,
    plan_id: &str,
    expected_action: OperationAction,
    helper_action: &str,
    mode: &str,
) -> Result<serde_json::Value, String> {
    if plan_id.len() != 64 || !plan_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("操作计划 ID 无效".to_owned());
    }
    let plan_path = cache_dir.join("plans").join(format!("{plan_id}.json"));
    let plan: OperationPlan = serde_json::from_slice(
        &fs::read(&plan_path).map_err(|error| format!("无法读取操作计划：{error}"))?,
    )
    .map_err(|error| format!("操作计划格式无效：{error}"))?;
    plan.verify_integrity()?;
    if plan.payload.action != expected_action {
        return Err("操作计划动作与请求不一致".to_owned());
    }
    let output = std::process::Command::new("/usr/bin/pkexec")
        .args(["/usr/libexec/umanager-helper", helper_action, "--plan"])
        .arg(&plan_path)
        .arg(mode)
        .output()
        .map_err(|error| format!("无法启动 Polkit dry-run：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "特权 helper dry-run 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("特权 helper 返回了无效结果：{error}"))
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
        assert!(run_dry_run(Path::new("/tmp/cache"), "../plan").is_err());
        assert!(run_dry_run(Path::new("/tmp/cache"), &"g".repeat(64)).is_err());
    }
}
