use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PLAN_SCHEMA_VERSION: u8 = 1;
pub const MAX_PLAN_LIFETIME_SECONDS: u64 = 15 * 60;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationPlan {
    pub plan_id: String,
    pub payload: PlanPayload,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlanPayload {
    pub schema_version: u8,
    pub action: OperationAction,
    pub application_id: String,
    pub package_name: String,
    pub installed_version: Option<String>,
    pub target_version: String,
    pub architecture: String,
    pub deb_path: String,
    pub sha256: String,
    pub size: u64,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationAction {
    InstallVerifiedDeb,
    InstallLocalDeb,
}

impl OperationPlan {
    pub fn new(payload: PlanPayload) -> Result<Self, String> {
        validate_payload_shape(&payload)?;
        let plan_id = payload_hash(&payload)?;
        Ok(Self { plan_id, payload })
    }

    pub fn verify_integrity(&self) -> Result<(), String> {
        validate_payload_shape(&self.payload)?;
        let expected = payload_hash(&self.payload)?;
        if !constant_time_ascii_eq(&self.plan_id, &expected) {
            return Err("operation plan integrity check failed".to_owned());
        }
        Ok(())
    }

    pub fn validate_time(&self, now_unix_seconds: u64) -> Result<(), String> {
        let created = self.payload.created_at_unix_seconds;
        let expires = self.payload.expires_at_unix_seconds;
        if expires <= created || expires - created > MAX_PLAN_LIFETIME_SECONDS {
            return Err("operation plan lifetime is invalid".to_owned());
        }
        if now_unix_seconds > expires {
            return Err("operation plan has expired".to_owned());
        }
        if created > now_unix_seconds.saturating_add(60) {
            return Err("operation plan creation time is in the future".to_owned());
        }
        Ok(())
    }
}

fn validate_payload_shape(payload: &PlanPayload) -> Result<(), String> {
    if payload.schema_version != PLAN_SCHEMA_VERSION {
        return Err("unsupported operation plan schema".to_owned());
    }
    for (name, value) in [
        ("applicationId", payload.application_id.as_str()),
        ("packageName", payload.package_name.as_str()),
        ("targetVersion", payload.target_version.as_str()),
        ("architecture", payload.architecture.as_str()),
        ("debPath", payload.deb_path.as_str()),
    ] {
        if value.is_empty() || value.contains('\0') {
            return Err(format!("operation plan field {name} is invalid"));
        }
    }
    if payload
        .installed_version
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.contains('\0'))
    {
        return Err("operation plan field installedVersion is invalid".to_owned());
    }
    if payload.size == 0 {
        return Err("operation plan file size is zero".to_owned());
    }
    if payload.sha256.len() != 64 || !payload.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("operation plan SHA-256 is invalid".to_owned());
    }
    Ok(())
}

fn payload_hash(payload: &PlanPayload) -> Result<String, String> {
    let encoded = serde_json::to_vec(payload)
        .map_err(|error| format!("cannot serialize operation plan: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn constant_time_ascii_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> PlanPayload {
        PlanPayload {
            schema_version: PLAN_SCHEMA_VERSION,
            action: OperationAction::InstallVerifiedDeb,
            application_id: "vscode".to_owned(),
            package_name: "code".to_owned(),
            installed_version: Some("1.0".to_owned()),
            target_version: "2.0".to_owned(),
            architecture: "amd64".to_owned(),
            deb_path: "/home/user/.cache/io.github.umanager.app/downloads/code_2.0_amd64.deb"
                .to_owned(),
            sha256: "a".repeat(64),
            size: 100,
            created_at_unix_seconds: 1_000,
            expires_at_unix_seconds: 1_900,
        }
    }

    #[test]
    fn detects_any_payload_mutation() {
        let mut plan = OperationPlan::new(payload()).unwrap();
        assert!(plan.verify_integrity().is_ok());
        plan.payload.target_version = "3.0".to_owned();
        assert!(plan.verify_integrity().is_err());
    }

    #[test]
    fn rejects_expired_and_overlong_plans() {
        let plan = OperationPlan::new(payload()).unwrap();
        assert!(plan.validate_time(1_500).is_ok());
        assert!(plan.validate_time(1_901).is_err());
        let mut overlong = payload();
        overlong.expires_at_unix_seconds += 1;
        let plan = OperationPlan::new(overlong).unwrap();
        assert!(plan.validate_time(1_500).is_err());
    }
}
