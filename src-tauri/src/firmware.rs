//! fwupd inventory and per-device immutable plans. Hardware operations use a
//! dedicated helper action; the signed .deb feed and downloader are unchanged.
use serde::Serialize;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::{io::ErrorKind, process::Stdio, time::Duration};
use tauri::{Emitter, Manager};

use umanager_plan::firmware::{Device, FirmwarePlan, FirmwareReport, Release, select_target};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FirmwareDevice {
    id: String,
    name: String,
    vendor: Option<String>,
    version: Option<String>,
    updatable: bool,
    supported: bool,
    needs_reboot: bool,
    needs_shutdown: bool,
    requires_ac: bool,
    problems: Vec<String>,
    update_error: Option<String>,
    releases: Vec<Release>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FirmwareState {
    available: bool,
    updates_checked: bool,
    devices: Vec<FirmwareDevice>,
    warnings: Vec<String>,
}

fn parse_inventory(bytes: &[u8]) -> Result<Vec<Device>, String> {
    umanager_plan::firmware::parse_devices(bytes)
}

// --force applies only to metadata age: explicitly clicking refresh should
// contact enabled remotes even within fwupd's normal 24-hour refresh interval.
const REFRESH_ARGUMENTS: &[&str] = &[
    "refresh",
    "--force",
    "--json",
    "--no-unreported-check",
    "--no-remote-check",
    "--no-reboot-check",
];
static METADATA_REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn check_refresh_result(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Result<(), String> {
    let message = serde_json::from_slice::<serde_json::Value>(stdout)
        .ok()
        .and_then(|value| {
            value
                .get("Error")
                .and_then(|error| error.get("Message"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    if code == Some(0) && message.is_none() {
        return Ok(());
    }
    let detail = message.unwrap_or_else(|| String::from_utf8_lossy(stderr).trim().to_owned());
    let detail: String = detail.chars().take(800).collect();
    Err(if detail.is_empty() {
        "固件元数据刷新失败，请检查网络连接及系统授权后重试".to_owned()
    } else {
        format!("固件元数据刷新失败：{detail}")
    })
}

/// Metadata only. No device IDs, firmware files, installation plans, helper
/// actions, remote enabling, or installation commands are accepted here.
#[tauri::command]
pub(crate) async fn refresh_firmware_metadata() -> Result<(), String> {
    let _guard = METADATA_REFRESH
        .try_lock()
        .map_err(|_| "固件元数据正在刷新，请稍后重试")?;
    let mut command = tokio::process::Command::new(umanager_plan::firmware::FWUPDMGR);
    command
        .args(REFRESH_ARGUMENTS)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command.spawn().map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "未安装 fwupd，无法联网刷新固件元数据".to_owned()
        } else {
            format!("无法启动固件元数据刷新：{error}")
        }
    })?;
    let output = tokio::time::timeout(Duration::from_secs(180), child.wait_with_output())
        .await
        .map_err(|_| "固件元数据刷新超时，请检查网络后重试；未执行固件安装".to_owned())?
        .map_err(|error| format!("无法完成固件元数据刷新：{error}"))?;
    check_refresh_result(output.status.code(), &output.stdout, &output.stderr)
}

async fn query(updates: bool) -> Result<Option<Vec<Device>>, String> {
    let mut command = tokio::process::Command::new("/usr/bin/fwupdmgr");
    command
        .args([
            if updates {
                "get-updates"
            } else {
                "get-devices"
            },
            "--json",
            "--no-metadata-check",
            "--no-unreported-check",
            "--no-remote-check",
        ])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法启动 fwupd 查询：{error}")),
    };
    let output = tokio::time::timeout(Duration::from_secs(45), child.wait_with_output())
        .await
        .map_err(|_| "fwupd 查询超时，请稍后重试".to_owned())?
        .map_err(|error| format!("fwupd 查询失败：{error}"))?;
    // fwupdmgr uses exit 2 for 'nothing to do'. Still require a valid inventory;
    // errors and incompatible JSON must never masquerade as 'no updates'.
    if !output.status.success() && output.status.code() != Some(2) {
        return Err(format!(
            "fwupd 查询失败：{}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(500)
                .collect::<String>()
        ));
    }
    parse_inventory(&output.stdout).map(Some)
}

fn merge(devices: Vec<Device>, updates: Vec<Device>) -> Vec<FirmwareDevice> {
    let mut updates: std::collections::HashMap<_, _> = updates
        .into_iter()
        .map(|device| (device.device_id.clone(), device))
        .collect();
    devices
        .into_iter()
        .map(|device| {
            let flag = |name: &str| device.flags.iter().any(|value| value == name);
            let update = updates.remove(&device.device_id);
            FirmwareDevice {
                updatable: flag("updatable"),
                supported: flag("supported"),
                needs_reboot: flag("needs-reboot"),
                needs_shutdown: flag("needs-shutdown"),
                requires_ac: flag("require-ac"),
                name: device.name.unwrap_or_else(|| "未命名设备".to_owned()),
                id: device.device_id,
                vendor: device.vendor,
                version: device.version,
                problems: device.problems,
                update_error: update
                    .as_ref()
                    .and_then(|item| item.update_error.clone())
                    .or(device.update_error),
                releases: update.map(|item| item.releases).unwrap_or_default(),
            }
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn get_firmware_state() -> Result<FirmwareState, String> {
    let Some(devices) = query(false).await? else {
        return Ok(FirmwareState {
            available: false,
            updates_checked: false,
            devices: vec![],
            warnings: vec![],
        });
    };
    let (updates, updates_checked, warnings) = match query(true).await {
        Ok(Some(updates)) => (updates, true, vec![]),
        Ok(None) => (vec![], false, vec!["查询期间 fwupdmgr 不再可用".to_owned()]),
        Err(error) => (vec![], false, vec![error]),
    };
    Ok(FirmwareState {
        available: true,
        updates_checked,
        devices: merge(devices, updates),
        warnings,
    })
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn persist_plan(directory: &Path, plan: &FirmwarePlan) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!("{}.json", plan.plan_id));
    static NEXT_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = directory.join(format!(
        ".{}-{}-{}.tmp",
        plan.plan_id,
        std::process::id(),
        NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let encoded = serde_json::to_vec(plan).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| format!("无法创建固件计划：{error}"))?;
    let outcome = (|| {
        file.write_all(&encoded)
            .map_err(|error| error.to_string())?;
        file.set_permissions(std::fs::Permissions::from_mode(0o400))
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        match std::fs::hard_link(&temporary, &path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata =
                    std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
                if !metadata.is_file()
                    || metadata.permissions().mode() & 0o222 != 0
                    || std::fs::read(&path).map_err(|error| error.to_string())? != encoded
                {
                    return Err("已有固件计划不匹配，拒绝覆盖".to_owned());
                }
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    })();
    let _ = std::fs::remove_file(temporary);
    outcome
}

#[tauri::command]
pub(crate) async fn prepare_firmware_update(
    app: tauri::AppHandle,
    device_id: String,
    version: String,
) -> Result<FirmwarePlan, String> {
    let devices = query(false).await?.ok_or("未安装 fwupd")?;
    let updates = query(true).await?.ok_or("未安装 fwupd")?;
    let target = select_target(&devices, &updates, &device_id, &version)?;
    let plan = FirmwarePlan::new(target, now())?;
    let directory = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("plans");
    persist_plan(&directory, &plan)?;
    Ok(plan)
}

async fn run_firmware_helper(
    app: tauri::AppHandle,
    plan_id: String,
    dry_run: bool,
) -> Result<FirmwareReport, String> {
    if plan_id.len() != 64 || !plan_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("固件计划 ID 无效".into());
    }
    let path = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("plans")
        .join(format!("{plan_id}.json"));
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let plan: FirmwarePlan = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    plan.verify(now())?;
    if plan.plan_id != plan_id {
        return Err("固件计划 ID 不匹配".into());
    }
    let progress: crate::operation_plan::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = app.emit("operation-progress", payload);
    });
    tauri::async_runtime::spawn_blocking(move || {
        let output = crate::operation_plan::run_privileged_helper(
            &plan_id,
            "install-device-firmware",
            &path,
            if dry_run { "--dry-run" } else { "--execute" },
            Some(progress),
        )?;
        let report: FirmwareReport = serde_json::from_slice(&output)
            .map_err(|error| format!("固件 helper 返回无效结果：{error}"))?;
        if report.plan_id != plan_id
            || report.dry_run != dry_run
            || !report.verified
            || report.target != plan.payload.target
        {
            return Err("固件 helper 返回的复核结果不匹配".into());
        }
        Ok(report)
    })
    .await
    .map_err(|error| format!("固件任务异常结束：{error}"))?
}

#[tauri::command]
pub(crate) async fn run_firmware_dry_run(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<FirmwareReport, String> {
    run_firmware_helper(app, plan_id, true).await
}

#[tauri::command]
pub(crate) async fn install_device_firmware(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<FirmwareReport, String> {
    run_firmware_helper(app, plan_id, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_errors_never_report_success_or_trigger_a_device_operation() {
        assert!(check_refresh_result(Some(0), b"", b"").is_ok());
        let error = br#"{"Error":{"Message":"Authorization denied"}}"#;
        assert!(
            check_refresh_result(Some(1), error, b"")
                .unwrap_err()
                .contains("Authorization denied")
        );
        assert!(check_refresh_result(Some(0), error, b"").is_err());
        assert!(check_refresh_result(Some(2), b"", b"metadata unavailable").is_err());
        assert!(check_refresh_result(None, b"", b"").is_err());
        assert_eq!(REFRESH_ARGUMENTS.first(), Some(&"refresh"));
        assert!(REFRESH_ARGUMENTS.contains(&"--no-remote-check"));
        assert!(!REFRESH_ARGUMENTS.iter().any(|argument| matches!(
            *argument,
            "install" | "local-install" | "update" | "enable-remote" | "--assume-yes"
        )));
    }

    #[test]
    fn plan_publication_is_read_only_atomic_and_does_not_overwrite() {
        let directory =
            std::env::temp_dir().join(format!("umanager-fw-plan-{}", std::process::id()));
        let target = umanager_plan::firmware::Target {
            device_id: "a".repeat(40),
            device_name: "SSD".into(),
            guids: vec!["hardware".into()],
            installed_version: "1".into(),
            target_version: "2".into(),
            remote_id: "lvfs".into(),
            checksums: vec!["b".repeat(64)],
            location: "https://fwupd.org/downloads/test.cab".into(),
            needs_reboot: true,
            needs_shutdown: false,
            requires_ac: true,
        };
        let plan = FirmwarePlan::new(target, 100).unwrap();
        persist_plan(&directory, &plan).unwrap();
        persist_plan(&directory, &plan).unwrap();
        let path = directory.join(format!("{}.json", plan.plan_id));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o222,
            0
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"tampered").unwrap();
        assert!(persist_plan(&directory, &plan).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"tampered");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn merges_updates_by_device_identity_and_preserves_requirements() {
        let devices = parse_inventory(br#"{"Devices":[{"DeviceId":"a","Name":"SSD","Version":"1","Flags":["updatable","require-ac","needs-reboot"],"Problems":["power-too-low"]},{"DeviceId":"b"}]}"#).unwrap();
        let updates = parse_inventory(br#"{"Devices":[{"DeviceId":"b","Releases":[{"Version":"9"}]},{"DeviceId":"a","Releases":[{"Version":"2","Description":"<p>Fix</p>"}]}]}"#).unwrap();
        let result = merge(devices, updates);
        assert_eq!(result[0].releases[0].version, "2");
        assert!(result[0].requires_ac && result[0].needs_reboot && result[0].updatable);
        assert_eq!(result[0].problems, ["power-too-low"]);
        assert_eq!(result[1].name, "未命名设备");
        assert!(!result[1].supported);
    }

    #[test]
    fn rejects_error_or_incompatible_json_instead_of_showing_no_updates() {
        assert!(parse_inventory(br#"{"Error":"service unavailable"}"#).is_err());
        assert!(parse_inventory(b"not json").is_err());
        assert!(parse_inventory(br#"{"Devices":[]}"#).unwrap().is_empty());
    }

    #[test]
    fn does_not_treat_installed_release_history_as_available_updates() {
        let devices =
            parse_inventory(br#"{"Devices":[{"DeviceId":"a","Releases":[{"Version":"old"}]}]}"#)
                .unwrap();
        assert!(merge(devices, vec![])[0].releases.is_empty());
    }
}
