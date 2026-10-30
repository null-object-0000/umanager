use super::*;
use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::time::{Duration, Instant};
use umanager_plan::firmware::{
    FWUPDMGR, FirmwarePlan, FirmwareReport, parse_devices, select_target,
};

const RUNTIME: &str = "/run/umanager-firmware";

fn read_plan(path: &Path, caller: &Caller, now: u64) -> Result<FirmwarePlan, String> {
    let plans = fs::canonicalize(caller.home.join(".cache/io.github.umanager.app/plans"))
        .map_err(|error| format!("无法定位固件计划目录：{error}"))?;
    let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
    if path.parent() != Some(plans.as_path()) {
        return Err("固件计划不在允许的目录".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != caller.uid
        || metadata.mode() & 0o222 != 0
        || metadata.len() > 65536
    {
        return Err("固件计划必须是当前用户拥有的只读普通文件".into());
    }
    let mut bytes = vec![];
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 65536 {
        return Err("固件计划过大".into());
    }
    let plan: FirmwarePlan = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    plan.verify(now)?;
    if path.file_name().and_then(|name| name.to_str()) != Some(&format!("{}.json", plan.plan_id)) {
        return Err("固件计划文件名不匹配".into());
    }
    Ok(plan)
}

fn secure_runtime() -> Result<PathBuf, String> {
    let path = PathBuf::from(RUNTIME);
    match fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.to_string()),
    }
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
        return Err("固件运行目录权限异常".into());
    }
    Ok(path)
}

fn query(updates: bool) -> Result<Vec<umanager_plan::firmware::Device>, String> {
    let mut child = clean_command(FWUPDMGR)
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
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法查询 fwupd：{error}"))?;
    fn reader(
        mut pipe: impl Read + Send + 'static,
    ) -> std::thread::JoinHandle<Result<Vec<u8>, String>> {
        std::thread::spawn(move || {
            let mut bytes = vec![];
            pipe.by_ref()
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            // Keep draining even on oversized output, to avoid blocking the child.
            std::io::copy(&mut pipe, &mut std::io::sink()).map_err(|error| error.to_string())?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err("fwupd 输出过大".into());
            }
            Ok(bytes)
        })
    }
    let stdout = reader(child.stdout.take().ok_or("无法读取 fwupd 输出")?);
    let stderr = reader(child.stderr.take().ok_or("无法读取 fwupd 错误")?);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(45) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("fwupd 复核超时；未开始固件安装".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let bytes = stdout.join().map_err(|_| "fwupd 输出读取失败")??;
    let errors = stderr.join().map_err(|_| "fwupd 错误读取失败")??;
    if !status.success() && status.code() != Some(2) {
        return Err(format!(
            "fwupd 复核失败：{}",
            String::from_utf8_lossy(&errors)
                .chars()
                .take(500)
                .collect::<String>()
        ));
    }
    parse_devices(&bytes)
}

/// Pure orchestration with an injected query/install boundary. Tests never
/// invoke fwupdmgr, pkexec, or a real device, including --execute tests.
fn verify_target(
    plan: &FirmwarePlan,
    devices: &[umanager_plan::firmware::Device],
    updates: &[umanager_plan::firmware::Device],
) -> Result<(), String> {
    let current = select_target(
        devices,
        updates,
        &plan.payload.target.device_id,
        &plan.payload.target.target_version,
    )?;
    if current != plan.payload.target {
        return Err("设备、版本或固件来源已变化，请重新创建计划并授权".into());
    }
    Ok(())
}

fn receipt_path(directory: &Path, uid: u32, plan: &FirmwarePlan) -> PathBuf {
    directory.join(format!("{uid}-{}.approved", plan.plan_id))
}

fn approve(directory: &Path, uid: u32, plan: &FirmwarePlan) -> Result<(), String> {
    let path = receipt_path(directory, uid, plan);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(plan.plan_id.as_bytes())
                .map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn consume(directory: &Path, uid: u32, plan: &FirmwarePlan) -> Result<(), String> {
    let path = receipt_path(directory, uid, plan);
    let text = fs::read_to_string(&path).map_err(|_| "尚未通过管理员复核，请先授权检查")?;
    if text != plan.plan_id {
        return Err("管理员复核记录不匹配".into());
    }
    fs::remove_file(path).map_err(|error| error.to_string())
}

pub(super) fn run(
    invocation: &Invocation,
    caller: &Caller,
    now: u64,
) -> Result<FirmwareReport, String> {
    let plan = read_plan(&invocation.plan_path, caller, now)?;
    let runtime = secure_runtime()?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(runtime.join("operation.lock"))
        .map_err(|error| error.to_string())?;
    // SAFETY: valid owned descriptor; advisory lock is held until File drops.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("已有固件操作正在进行，请等待完成".into());
    }
    workflow(
        &plan,
        invocation.mode,
        &runtime,
        caller.uid,
        unix_timestamp,
        query,
        |plan| install_pinned(&runtime, plan),
    )
}

fn install_pinned(runtime: &Path, plan: &FirmwarePlan) -> Result<(), String> {
    use umanager_plan::firmware::{archive_filename, verify_archive_checksum};
    let stage = runtime.join(format!("{}-{}", plan.plan_id, std::process::id()));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&stage)
        .map_err(|error| error.to_string())?;
    struct Stage(PathBuf);
    impl Drop for Stage {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _stage_cleanup = Stage(stage.clone());
    let target = &plan.payload.target;
    let filename = archive_filename(&target.location)?;
    let mut download = clean_command(FWUPDMGR);
    download
        .current_dir(&stage)
        .args([
            "download",
            &target.location,
            "--json",
            "--no-metadata-check",
            "--no-unreported-check",
            "--no-remote-check",
            "--no-reboot-check",
        ])
        .stdin(Stdio::null());
    execute_streaming(download, "固件下载")?;
    let archive = stage.join(filename);
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&archive)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.len() == 0
        || metadata.len() > 1024 * 1024 * 1024
    {
        return Err("固件包不是 root 拥有的普通文件，或大小异常".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    verify_archive_checksum(target, &format!("{:x}", digest.finalize()))?;
    // A slow download must not authorize a stale device or an expired plan.
    verify_target(plan, &query(false)?, &query(true)?)?;
    plan.verify(unix_timestamp())?;
    emit_progress(
        "phase",
        "system",
        "固件包校验和设备复核通过，开始写入已确认设备；请勿断电",
    );
    let mut command = clean_command(FWUPDMGR);
    command
        .current_dir(&stage)
        .args(plan.install_arguments(&archive))
        .stdin(Stdio::null());
    // Never terminate a potentially active firmware write on a timeout.
    execute_streaming(command, "固件写入")
}

fn workflow(
    plan: &FirmwarePlan,
    mode: ExecutionMode,
    runtime: &Path,
    uid: u32,
    clock: impl Fn() -> u64,
    mut query: impl FnMut(bool) -> Result<Vec<umanager_plan::firmware::Device>, String>,
    mut install: impl FnMut(&FirmwarePlan) -> Result<(), String>,
) -> Result<FirmwareReport, String> {
    plan.verify(clock())?;
    if mode == ExecutionMode::Execute {
        consume(runtime, uid, plan)?;
    }
    emit_progress(
        "phase",
        "system",
        "正在独立复核设备、当前版本、目标版本和固件校验信息",
    );
    verify_target(plan, &query(false)?, &query(true)?)?;
    plan.verify(clock())?;
    if mode == ExecutionMode::DryRun {
        approve(runtime, uid, plan)?;
    } else {
        // Prevent fwupdmgr's local-file compatibility fallback. Production uses
        // a root-owned directory, so another user cannot create this file.
        if runtime.join(&plan.payload.target.device_id).exists() {
            return Err("固件设备 ID 与本地路径冲突".into());
        }
        emit_progress(
            "phase",
            "system",
            "开始下载并更新所选设备；请勿断电、拔出设备或强制退出",
        );
        install(plan)?;
    }
    Ok(FirmwareReport {
        plan_id: plan.plan_id.clone(),
        dry_run: mode == ExecutionMode::DryRun,
        verified: true,
        target: plan.payload.target.clone(),
        message: if mode == ExecutionMode::DryRun {
            "管理员复核通过，尚未下载或安装固件。请核对设备和版本后再次确认。"
        } else {
            "fwupd 已完成更新操作；如设备需要重启或关机，请保存工作后手动操作。"
        }
        .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        FirmwarePlan,
        Vec<umanager_plan::firmware::Device>,
        Vec<umanager_plan::firmware::Device>,
    ) {
        let devices = parse_devices(format!(r#"{{"Devices":[{{"DeviceId":"{}","Name":"SSD","Version":"1","Guid":["g"],"Flags":["supported","updatable"]}}]}}"#, "a".repeat(40)).as_bytes()).unwrap();
        let updates = parse_devices(format!(r#"{{"Devices":[{{"DeviceId":"{}","Releases":[{{"Version":"2","RemoteId":"lvfs","Locations":["https://fwupd.org/downloads/test.cab"],"Checksum":["{}"],"Flags":["trusted-metadata","is-upgrade"]}}]}}]}}"#, "a".repeat(40), "b".repeat(64)).as_bytes()).unwrap();
        let target = select_target(&devices, &updates, &"a".repeat(40), "2").unwrap();
        (FirmwarePlan::new(target, 100).unwrap(), devices, updates)
    }
    #[test]
    fn revalidation_blocks_device_version_checksum_and_power_changes() {
        let (plan, mut devices, mut updates) = fixture();
        assert!(verify_target(&plan, &devices, &updates).is_ok());
        devices[0].version = Some("3".into());
        assert!(verify_target(&plan, &devices, &updates).is_err());
        devices[0].version = Some("1".into());
        updates[0].releases[0].checksum = vec!["c".repeat(64)];
        assert!(verify_target(&plan, &devices, &updates).is_err());
        updates[0].releases[0].checksum = vec!["b".repeat(64)];
        devices[0].problems.push("require-ac-power".into());
        assert!(verify_target(&plan, &devices, &updates).is_err());
        assert!(verify_target(&plan, &[], &updates).is_err());
    }
    #[test]
    fn full_workflow_never_writes_during_dry_run_and_executes_only_once_after_confirmation() {
        let (plan, devices, updates) = fixture();
        let dir = std::env::temp_dir().join(format!("umanager-fw-workflow-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let query = |updates_requested| {
            Ok(if updates_requested {
                updates.clone()
            } else {
                devices.clone()
            })
        };
        let mut writes = vec![];
        let mut install = |plan: &FirmwarePlan| {
            writes.push(plan.install_arguments(Path::new("/root/firmware.cab")));
            Ok(())
        };
        assert!(
            workflow(
                &plan,
                ExecutionMode::Execute,
                &dir,
                1001,
                || 101,
                query,
                &mut install
            )
            .is_err()
        );
        assert!(
            workflow(
                &plan,
                ExecutionMode::DryRun,
                &dir,
                1001,
                || 101,
                query,
                &mut install
            )
            .unwrap()
            .dry_run
        );
        assert!(writes.is_empty());
        let mut install = |plan: &FirmwarePlan| {
            writes.push(plan.install_arguments(Path::new("/root/firmware.cab")));
            Ok(())
        };
        let report = workflow(
            &plan,
            ExecutionMode::Execute,
            &dir,
            1001,
            || 102,
            query,
            &mut install,
        )
        .unwrap();
        assert!(!report.dry_run);
        assert!(
            workflow(
                &plan,
                ExecutionMode::Execute,
                &dir,
                1001,
                || 102,
                query,
                &mut install
            )
            .is_err()
        );
        assert_eq!(writes.len(), 1);
        assert_eq!(
            &writes[0][..3],
            &["local-install", "/root/firmware.cab", &"a".repeat(40)]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn expired_or_changed_plan_after_dry_run_does_not_call_install() {
        let (plan, mut devices, updates) = fixture();
        let dir = std::env::temp_dir().join(format!("umanager-fw-changed-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        approve(&dir, 1001, &plan).unwrap();
        let never_install = |_: &FirmwarePlan| -> Result<(), String> {
            panic!("must not write firmware");
        };
        assert!(
            workflow(
                &plan,
                ExecutionMode::Execute,
                &dir,
                1001,
                || 1001,
                |_| panic!("expired before query"),
                never_install
            )
            .is_err()
        );
        devices[0].version = Some("3".into());
        assert!(
            workflow(
                &plan,
                ExecutionMode::Execute,
                &dir,
                1001,
                || 101,
                |u| Ok(if u { updates.clone() } else { devices.clone() }),
                never_install
            )
            .is_err()
        );
        assert!(consume(&dir, 1001, &plan).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn execute_requires_matching_user_dry_run_and_consumes_receipt_once() {
        let (plan, _, _) = fixture();
        let dir = std::env::temp_dir().join(format!("umanager-fw-receipt-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(consume(&dir, 1001, &plan).is_err());
        approve(&dir, 1001, &plan).unwrap();
        assert!(consume(&dir, 1002, &plan).is_err());
        consume(&dir, 1001, &plan).unwrap();
        assert!(consume(&dir, 1001, &plan).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
