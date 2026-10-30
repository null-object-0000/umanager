//! Separate hardware-operation protocol; does not change the .deb/feed schema.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FWUPDMGR: &str = "/usr/bin/fwupdmgr";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all(deserialize = "PascalCase", serialize = "camelCase"))]
pub struct Device {
    pub device_id: String,
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub version: Option<String>,
    #[serde(default, rename(deserialize = "Guid"))]
    pub guids: Vec<String>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub problems: Vec<String>,
    pub update_error: Option<String>,
    pub update_state: Option<u32>,
    #[serde(default)]
    pub releases: Vec<Release>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all(deserialize = "PascalCase", serialize = "camelCase"))]
pub struct Release {
    pub version: String,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub remote_id: Option<String>,
    #[serde(default)]
    pub checksum: Vec<String>,
    #[serde(default)]
    pub locations: Vec<String>,
    #[serde(default)]
    pub flags: Vec<String>,
}

pub fn parse_devices(bytes: &[u8]) -> Result<Vec<Device>, String> {
    #[derive(Deserialize)]
    struct Inventory {
        #[serde(rename = "Devices")]
        devices: Vec<Device>,
    }
    serde_json::from_slice::<Inventory>(bytes)
        .map(|value| value.devices)
        .map_err(|_| "fwupd 返回了无法识别的数据，请检查 fwupd 版本与服务状态".to_owned())
}

pub fn valid_device_id(id: &str) -> bool {
    id.len() == 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub device_id: String,
    pub device_name: String,
    pub guids: Vec<String>,
    pub installed_version: String,
    pub target_version: String,
    pub remote_id: String,
    pub checksums: Vec<String>,
    pub location: String,
    pub needs_reboot: bool,
    pub needs_shutdown: bool,
    pub requires_ac: bool,
}

pub fn select_target(
    devices: &[Device],
    updates: &[Device],
    id: &str,
    version: &str,
) -> Result<Target, String> {
    if !valid_device_id(id) {
        return Err("固件设备 ID 无效".to_owned());
    }
    let device = devices
        .iter()
        .find(|device| device.device_id == id)
        .ok_or("设备已断开，请重新检查")?;
    let has = |flag: &str| device.flags.iter().any(|value| value == flag);
    if !has("updatable")
        || !has("supported")
        || !device.problems.is_empty()
        || device
            .update_error
            .as_ref()
            .is_some_and(|value| !value.is_empty())
        || matches!(device.update_state, Some(1 | 4 | 6))
    {
        return Err("设备当前不能更新，或有待完成的更新；请处理设备限制后重新检查".to_owned());
    }
    let installed_version = device.version.clone().ok_or("设备未提供当前固件版本")?;
    if installed_version == version {
        return Err("不允许重装当前固件".to_owned());
    }
    let update = updates
        .iter()
        .find(|device| device.device_id == id)
        .ok_or("所选设备已无可用更新")?;
    let releases: Vec<_> = update
        .releases
        .iter()
        .filter(|release| release.version == version)
        .collect();
    if releases.len() != 1 {
        return Err("目标版本已变化或不唯一，请重新检查".to_owned());
    }
    let release = releases[0];
    if !release.flags.iter().any(|flag| flag == "trusted-metadata")
        || !release.flags.iter().any(|flag| flag == "is-upgrade")
        || release.flags.iter().any(|flag| {
            matches!(
                flag.as_str(),
                "is-downgrade" | "blocked-version" | "blocked-approval"
            )
        })
    {
        return Err("固件缺少可信元数据，或被 fwupd 阻止安装".to_owned());
    }
    let remote_id = release
        .remote_id
        .clone()
        .filter(|value| !value.is_empty())
        .ok_or("固件未关联系统配置的更新源")?;
    let mut checksums = release.checksum.clone();
    checksums.sort();
    checksums.dedup();
    if !checksums
        .iter()
        .any(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("固件未提供 SHA-256 校验信息".to_owned());
    }
    let mut guids = device.guids.clone();
    guids.sort();
    guids.dedup();
    if guids.is_empty() {
        return Err("设备未提供硬件标识".to_owned());
    }
    let location = release
        .locations
        .iter()
        .find(|location| archive_filename(location).is_ok())
        .cloned()
        .ok_or("固件没有可直接下载的 HTTPS cabinet 地址，暂不支持安装")?;
    Ok(Target {
        device_id: id.to_owned(),
        device_name: device
            .name
            .clone()
            .unwrap_or_else(|| "未命名设备".to_owned()),
        guids,
        installed_version,
        target_version: version.to_owned(),
        remote_id,
        checksums,
        location,
        needs_reboot: has("needs-reboot"),
        needs_shutdown: has("needs-shutdown"),
        requires_ac: has("require-ac"),
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirmwarePayload {
    pub schema_version: u8,
    pub action: String,
    pub target: Target,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirmwarePlan {
    pub plan_id: String,
    pub payload: FirmwarePayload,
}

impl FirmwarePlan {
    pub fn new(target: Target, now: u64) -> Result<Self, String> {
        let payload = FirmwarePayload {
            schema_version: 1,
            action: "installDeviceFirmware".to_owned(),
            target,
            created_at_unix_seconds: now,
            expires_at_unix_seconds: now + super::MAX_PLAN_LIFETIME_SECONDS,
        };
        let plan_id = hash(&payload)?;
        let plan = Self { plan_id, payload };
        plan.verify(now)?;
        Ok(plan)
    }
    pub fn verify(&self, now: u64) -> Result<(), String> {
        let target = &self.payload.target;
        if self.payload.schema_version != 1
            || self.payload.action != "installDeviceFirmware"
            || !valid_device_id(&target.device_id)
            || self.plan_id != hash(&self.payload)?
        {
            return Err("固件操作计划无效或已被修改".to_owned());
        }
        for version in [&target.installed_version, &target.target_version] {
            if version.is_empty()
                || version.len() > 128
                || version.starts_with('-')
                || !version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
            {
                return Err("固件版本格式无效".to_owned());
            }
        }
        archive_filename(&target.location)?;
        super::validate_plan_time(
            self.payload.created_at_unix_seconds,
            self.payload.expires_at_unix_seconds,
            now,
        )
    }
    /// Always uses exactly one device and the root-staged, checksum-pinned CAB.
    /// No force, downgrade, reinstall, remote mutation, or reboot options.
    pub fn install_arguments(&self, archive: &std::path::Path) -> Vec<String> {
        [
            "local-install",
            archive
                .to_str()
                .expect("internal firmware staging path is UTF-8"),
            &self.payload.target.device_id,
            "--json",
            "--no-reboot-check",
            "--no-unreported-check",
            "--no-metadata-check",
            "--no-remote-check",
            "--no-device-prompt",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
}

/// The download helper writes the URL basename into a root-owned staging
/// directory. Reject path tricks, credentials, query strings and relative URLs.
pub fn archive_filename(location: &str) -> Result<String, String> {
    let url = url::Url::parse(location).map_err(|_| "固件下载地址无效")?;
    let name = url
        .path_segments()
        .and_then(|mut parts| parts.next_back())
        .unwrap_or_default();
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || name.len() > 200
        || !name.ends_with(".cab")
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err("固件下载地址必须是可信元数据中的直接 HTTPS cabinet 地址".into());
    }
    Ok(name.to_owned())
}

pub fn verify_archive_checksum(target: &Target, sha256: &str) -> Result<(), String> {
    if !target
        .checksums
        .iter()
        .any(|checksum| checksum.len() == 64 && checksum.eq_ignore_ascii_case(sha256))
    {
        return Err("固件包 SHA-256 与已确认计划不匹配，拒绝写入设备".into());
    }
    Ok(())
}

fn hash(payload: &FirmwarePayload) -> Result<String, String> {
    let bytes = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareReport {
    pub plan_id: String,
    pub dry_run: bool,
    pub verified: bool,
    pub target: Target,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn fixture() -> (Vec<Device>, Vec<Device>) {
        let id = "a".repeat(40);
        let devices = parse_devices(serde_json::to_string(&serde_json::json!({"Devices":[{"DeviceId":id,"Name":"SSD","Version":"1","Guid":["hardware-guid"],"Flags":["supported","updatable","require-ac","needs-reboot"]}]})).unwrap().as_bytes()).unwrap();
        let updates = parse_devices(serde_json::to_string(&serde_json::json!({"Devices":[{"DeviceId":id,"Releases":[{"Version":"2","RemoteId":"lvfs","Locations":["https://fwupd.org/downloads/test.cab"],"Checksum":["b".repeat(64)],"Flags":["trusted-metadata","is-upgrade"]}]}]})).unwrap().as_bytes()).unwrap();
        (devices, updates)
    }
    #[test]
    fn rejects_untrusted_blocked_disconnected_and_ambiguous_targets() {
        let (mut devices, mut updates) = fixture();
        let id = "a".repeat(40);
        assert!(select_target(&devices, &updates, &id, "2").is_ok());
        assert!(select_target(&devices, &updates, &"c".repeat(40), "2").is_err());
        devices[0].problems.push("power-too-low".into());
        assert!(select_target(&devices, &updates, &id, "2").is_err());
        devices[0].problems.clear();
        updates[0].releases[0].flags.clear();
        assert!(select_target(&devices, &updates, &id, "2").is_err());
        updates[0].releases[0].flags.push("trusted-metadata".into());
        let duplicate = updates[0].releases[0].clone();
        updates[0].releases.push(duplicate);
        assert!(select_target(&devices, &updates, &id, "2").is_err());
    }
    #[test]
    fn locks_target_rejects_tampering_expiry_and_argument_injection() {
        let (devices, updates) = fixture();
        let target = select_target(&devices, &updates, &"a".repeat(40), "2").unwrap();
        let mut plan = FirmwarePlan::new(target.clone(), 100).unwrap();
        assert!(plan.verify(1001).is_err());
        plan.payload.target.target_version = "3".into();
        assert!(plan.verify(101).is_err());
        for id in ["*", "--force", "https://bad.invalid", "../payload.cab"] {
            let mut target = target.clone();
            target.device_id = id.into();
            assert!(FirmwarePlan::new(target, 100).is_err());
        }
        let mut target = target;
        target.target_version = "--allow-older".into();
        assert!(FirmwarePlan::new(target, 100).is_err());
    }
    #[test]
    fn rejects_changed_artifact_and_unsafe_download_locations() {
        let (devices, updates) = fixture();
        let target = select_target(&devices, &updates, &"a".repeat(40), "2").unwrap();
        assert!(verify_archive_checksum(&target, &"b".repeat(64)).is_ok());
        assert!(verify_archive_checksum(&target, &"c".repeat(64)).is_err());
        for location in [
            "file:///tmp/fw.cab",
            "http://fwupd.org/fw.cab",
            "https://a:b@fwupd.org/fw.cab",
            "https://fwupd.org/%2e%2e.cab",
            "https://fwupd.org/fw.cab?file=x",
            "../fw.cab",
        ] {
            assert!(archive_filename(location).is_err(), "{location}");
        }
        assert_eq!(
            archive_filename("https://fwupd.org/downloads/fw.cab").unwrap(),
            "fw.cab"
        );
    }

    #[test]
    fn cannot_expand_to_all_devices_or_silently_retarget_version() {
        let (devices, updates) = fixture();
        let target = select_target(&devices, &updates, &"a".repeat(40), "2").unwrap();
        let plan = FirmwarePlan::new(target, 100).unwrap();
        let args =
            plan.install_arguments(std::path::Path::new("/run/umanager-firmware/firmware.cab"));
        assert_eq!(
            &args[..3],
            &[
                "local-install",
                "/run/umanager-firmware/firmware.cab",
                &"a".repeat(40)
            ]
        );
        assert!(args.iter().any(|value| value == "--no-reboot-check"));
        assert!(!args.iter().any(|value| {
            [
                "--force",
                "--allow-older",
                "--allow-reinstall",
                "--no-safety-check",
            ]
            .contains(&value.as_str())
        }));
    }
}
