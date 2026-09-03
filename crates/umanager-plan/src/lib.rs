use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PLAN_SCHEMA_VERSION: u8 = 3;
pub const MAX_PLAN_LIFETIME_SECONDS: u64 = 15 * 60;
const MAX_CATALOG_AUTH_BYTES: usize = 256 * 1024;

/// A non-central source feed reference carried in a v3 plan, endorsed by the
/// central feed's signature (DESIGN-multi-source.md §5). The helper verifies
/// `source_endorsement` (central key over these bytes) then `source_catalog_*`
/// (the source key over the source's catalog) before authorizing a feed-added
/// application that came from a third-party or non-central source.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub source_id: String,
    pub feed_url: String,
    pub public_key_hex: String,
}

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
    /// Optional signed catalog carried only for applications that were added by
    /// the metadata feed (i.e. not compiled into the privileged helper). The
    /// helper verifies `catalog_signature` over `catalog_json` before accepting
    /// the application as allowlisted. Used for central-source applications.
    #[serde(default)]
    pub catalog_json: Option<String>,
    #[serde(default)]
    pub catalog_signature: Option<String>,
    /// v3: source chain for an application added by a non-central source. The
    /// helper verifies `source_endorsement` (central signature over `source_ref`)
    /// then `source_catalog_signature` (the source key over `source_catalog_json`).
    #[serde(default)]
    pub source_ref: Option<SourceRef>,
    #[serde(default)]
    pub source_endorsement: Option<String>,
    #[serde(default)]
    pub source_catalog_json: Option<String>,
    #[serde(default)]
    pub source_catalog_signature: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationAction {
    InstallVerifiedDeb,
    InstallVerifiedWebsiteDeb,
    InstallLocalDeb,
    InstallSelfUpdate,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlan {
    pub plan_id: String,
    pub payload: RemovalPlanPayload,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemovalPlanPayload {
    pub schema_version: u8,
    pub action: RemovalAction,
    pub application_id: String,
    pub package_name: String,
    pub installed_version: String,
    pub architecture: String,
    pub created_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    /// Signed catalog carried for feed-added applications (see `PlanPayload`).
    #[serde(default)]
    pub catalog_json: Option<String>,
    #[serde(default)]
    pub catalog_signature: Option<String>,
    /// v3: source chain for an application added by a non-central source
    /// (see `PlanPayload`).
    #[serde(default)]
    pub source_ref: Option<SourceRef>,
    #[serde(default)]
    pub source_endorsement: Option<String>,
    #[serde(default)]
    pub source_catalog_json: Option<String>,
    #[serde(default)]
    pub source_catalog_signature: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RemovalAction {
    RemoveManagedPackage,
    RemoveUmanager,
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

impl RemovalPlan {
    pub fn new(payload: RemovalPlanPayload) -> Result<Self, String> {
        validate_removal_payload_shape(&payload)?;
        let plan_id = removal_payload_hash(&payload)?;
        Ok(Self { plan_id, payload })
    }

    pub fn verify_integrity(&self) -> Result<(), String> {
        validate_removal_payload_shape(&self.payload)?;
        let expected = removal_payload_hash(&self.payload)?;
        if !constant_time_ascii_eq(&self.plan_id, &expected) {
            return Err("removal plan integrity check failed".to_owned());
        }
        Ok(())
    }

    pub fn validate_time(&self, now_unix_seconds: u64) -> Result<(), String> {
        validate_plan_time(
            self.payload.created_at_unix_seconds,
            self.payload.expires_at_unix_seconds,
            now_unix_seconds,
        )
    }
}

fn validate_payload_shape(payload: &PlanPayload) -> Result<(), String> {
    if !matches!(payload.schema_version, 2 | 3) {
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
    validate_catalog_auth(
        payload.catalog_json.as_deref(),
        payload.catalog_signature.as_deref(),
        "operation plan",
    )?;
    validate_source_chain(
        payload.source_ref.as_ref(),
        payload.source_endorsement.as_deref(),
        payload.source_catalog_json.as_deref(),
        payload.source_catalog_signature.as_deref(),
        "operation plan",
    )?;
    Ok(())
}

/// Validate the v3 source chain: `source_ref` and its endorsement + source
/// catalog must all be present together (or all absent). When present, the
/// source id / feed URL / public key and the two signatures are format-checked
/// here; the helper still performs the actual cryptographic verification.
fn validate_source_chain(
    source_ref: Option<&SourceRef>,
    endorsement: Option<&str>,
    catalog_json: Option<&str>,
    catalog_signature: Option<&str>,
    kind: &str,
) -> Result<(), String> {
    let parts = [endorsement.is_some(), catalog_json.is_some(), catalog_signature.is_some()];
    let any = parts.iter().any(|value| *value);
    let all = parts.iter().all(|value| *value);
    // All four chain fields must come together: either a complete chain, or none.
    if (source_ref.is_some() != all) || (!all && any) {
        return Err(format!("{kind} source chain fields are incomplete"));
    }
    if !all {
        return Ok(());
    }
    let source = source_ref.expect("all source chain parts present");
    if source.source_id.is_empty() || source.source_id.contains('\0') {
        return Err(format!("{kind} sourceRef id is invalid"));
    }
    if !source.feed_url.starts_with("https://") {
        return Err(format!("{kind} sourceRef feedUrl must be HTTPS"));
    }
    if source.public_key_hex.len() != 64 || !source.public_key_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{kind} sourceRef publicKeyHex is invalid"));
    }
    let endorsement = endorsement.expect("all source chain parts present");
    if endorsement.len() != 128 || !endorsement.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{kind} source endorsement is invalid"));
    }
    validate_catalog_auth(catalog_json, catalog_signature, kind)?;
    Ok(())
}

fn validate_catalog_auth(json: Option<&str>, signature: Option<&str>, kind: &str) -> Result<(), String> {
    if json.is_some() != signature.is_some() {
        return Err(format!("{kind} catalog auth fields are incomplete"));
    }
    if let Some(json) = json {
        if json.len() > MAX_CATALOG_AUTH_BYTES || json.contains('\0') {
            return Err(format!("{kind} catalog payload is invalid"));
        }
    }
    if let Some(signature) = signature {
        if signature.len() != 128 || !signature.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{kind} catalog signature is invalid"));
        }
    }
    Ok(())
}

fn validate_removal_payload_shape(payload: &RemovalPlanPayload) -> Result<(), String> {
    if !matches!(payload.schema_version, 2 | 3) {
        return Err("unsupported removal plan schema".to_owned());
    }
    for (name, value) in [
        ("applicationId", payload.application_id.as_str()),
        ("packageName", payload.package_name.as_str()),
        ("installedVersion", payload.installed_version.as_str()),
        ("architecture", payload.architecture.as_str()),
    ] {
        if value.is_empty() || value.contains('\0') {
            return Err(format!("removal plan field {name} is invalid"));
        }
    }
    if !valid_package_name(&payload.package_name) {
        return Err("removal plan package name is invalid".to_owned());
    }
    validate_catalog_auth(
        payload.catalog_json.as_deref(),
        payload.catalog_signature.as_deref(),
        "removal plan",
    )?;
    validate_source_chain(
        payload.source_ref.as_ref(),
        payload.source_endorsement.as_deref(),
        payload.source_catalog_json.as_deref(),
        payload.source_catalog_signature.as_deref(),
        "removal plan",
    )?;
    Ok(())
}

fn valid_package_name(value: &str) -> bool {
    value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')
        })
}

fn payload_hash(payload: &PlanPayload) -> Result<String, String> {
    let encoded = serde_json::to_vec(payload)
        .map_err(|error| format!("cannot serialize operation plan: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn removal_payload_hash(payload: &RemovalPlanPayload) -> Result<String, String> {
    let encoded = serde_json::to_vec(payload)
        .map_err(|error| format!("cannot serialize removal plan: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn validate_plan_time(created: u64, expires: u64, now: u64) -> Result<(), String> {
    if expires <= created || expires - created > MAX_PLAN_LIFETIME_SECONDS {
        return Err("plan lifetime is invalid".to_owned());
    }
    if now > expires {
        return Err("plan has expired".to_owned());
    }
    if created > now.saturating_add(60) {
        return Err("plan creation time is in the future".to_owned());
    }
    Ok(())
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
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
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

    #[test]
    fn removal_plan_detects_mutation_and_rejects_option_like_packages() {
        let payload = RemovalPlanPayload {
            schema_version: PLAN_SCHEMA_VERSION,
            action: RemovalAction::RemoveManagedPackage,
            application_id: "wechat".to_owned(),
            package_name: "wechat".to_owned(),
            installed_version: "4.1.1.8".to_owned(),
            architecture: "amd64".to_owned(),
            created_at_unix_seconds: 1_000,
            expires_at_unix_seconds: 1_900,
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
        };
        let mut plan = RemovalPlan::new(payload.clone()).unwrap();
        assert!(plan.verify_integrity().is_ok());
        assert!(plan.validate_time(1_500).is_ok());
        plan.payload.package_name = "other".to_owned();
        assert!(plan.verify_integrity().is_err());

        let mut invalid = payload;
        invalid.package_name = "--force-all".to_owned();
        assert!(RemovalPlan::new(invalid).is_err());
    }

    #[test]
    fn v3_source_chain_requires_complete_chain_and_valid_ref() {
        let mut p = payload();
        p.source_ref = Some(SourceRef {
            source_id: "tencent".to_owned(),
            feed_url: "https://example.com/feed.tencent.json".to_owned(),
            public_key_hex: "a".repeat(64),
        });
        p.source_endorsement = Some("b".repeat(128));
        p.source_catalog_json = Some(r#"[{"applicationId":"qq"}]"#.to_owned());
        p.source_catalog_signature = Some("c".repeat(128));
        assert!(OperationPlan::new(p.clone()).is_ok(), "complete source chain accepted");
        assert!(OperationPlan::new(p.clone()).unwrap().verify_integrity().is_ok());

        let mut incomplete = p.clone();
        incomplete.source_catalog_signature = None;
        assert!(OperationPlan::new(incomplete).is_err(), "incomplete source chain rejected");

        let mut bad_url = p.clone();
        bad_url.source_ref = Some(SourceRef {
            source_id: "tencent".to_owned(),
            feed_url: "http://insecure.example/f.json".to_owned(),
            public_key_hex: "a".repeat(64),
        });
        assert!(OperationPlan::new(bad_url).is_err(), "http feedUrl rejected");

        let mut removal = RemovalPlanPayload {
            schema_version: PLAN_SCHEMA_VERSION,
            action: RemovalAction::RemoveManagedPackage,
            application_id: "qq".to_owned(),
            package_name: "linuxqq".to_owned(),
            installed_version: "3.2".to_owned(),
            architecture: "amd64".to_owned(),
            created_at_unix_seconds: 1_000,
            expires_at_unix_seconds: 1_900,
            catalog_json: None,
            catalog_signature: None,
            source_ref: p.source_ref.clone(),
            source_endorsement: p.source_endorsement.clone(),
            source_catalog_json: p.source_catalog_json.clone(),
            source_catalog_signature: p.source_catalog_signature.clone(),
        };
        assert!(RemovalPlan::new(removal.clone()).is_ok());
        removal.source_ref = None;
        assert!(RemovalPlan::new(removal).is_err(), "removal chain without sourceRef rejected");
    }
}
