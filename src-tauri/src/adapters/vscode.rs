use crate::scanner::{self, ManagedPackage, SourceKind, UpdateState};
use serde::Serialize;

pub mod download;

pub(super) const PACKAGE_NAME: &str = "code";
pub(super) const ARCHITECTURE: &str = "amd64";
pub(super) const REPOSITORY_HOST: &str = "packages.microsoft.com";
const STABLE_DOWNLOAD_ENDPOINT: &str =
    "https://update.code.visualstudio.com/latest/linux-deb-x64/stable";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VscodeDetails {
    application_id: &'static str,
    display_name: String,
    package_name: String,
    installed_version: String,
    candidate_version: Option<String>,
    architecture: String,
    support_level: &'static str,
    trust_state: TrustState,
    update_state: UpdateState,
    selected_path: UpdatePath,
    fallback_endpoint: &'static str,
    evidence: Vec<TrustEvidence>,
    verification_plan: Vec<VerificationCheck>,
    operation_plan: Vec<PlanStep>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum TrustState {
    Trusted,
    NeedsReview,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePath {
    kind: UpdatePathKind,
    label: &'static str,
    endpoint: String,
    reason: &'static str,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum UpdatePathKind {
    OfficialRepository,
    StableDownloadEndpoint,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrustEvidence {
    label: &'static str,
    actual: String,
    expected: &'static str,
    passed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerificationCheck {
    label: &'static str,
    expected: String,
    state: CheckState,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum CheckState {
    Passed,
    Planned,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanStep {
    order: u8,
    action: &'static str,
    detail: String,
    state: PlanState,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum PlanState {
    Complete,
    Planned,
    NotRequired,
}

pub fn load_details() -> Result<VscodeDetails, String> {
    let result = scanner::scan()?;
    let package = result
        .packages
        .iter()
        .find(|package| package.package_name == PACKAGE_NAME)
        .ok_or_else(|| "未检测到已安装的 Visual Studio Code（软件包 code）".to_owned())?;
    Ok(build_details(package))
}

fn build_details(package: &ManagedPackage) -> VscodeDetails {
    let package_matches = package.package_name == PACKAGE_NAME;
    let architecture_matches = package.architecture == ARCHITECTURE;
    let official_repository = package
        .source_url
        .as_deref()
        .filter(|url| url_has_https_host(url, REPOSITORY_HOST));
    let repository_is_official = official_repository.is_some()
        && matches!(package.source_kind, SourceKind::OfficialRepository);
    let trusted = package_matches && architecture_matches && repository_is_official;

    let selected_path = if let Some(repository) = official_repository {
        UpdatePath {
            kind: UpdatePathKind::OfficialRepository,
            label: "Microsoft 官方 APT 仓库",
            endpoint: repository.to_owned(),
            reason: "已配置官方仓库，按照下载优先级直接使用 APT 候选版本。",
        }
    } else {
        UpdatePath {
            kind: UpdatePathKind::StableDownloadEndpoint,
            label: "VS Code 官方稳定版接口",
            endpoint: STABLE_DOWNLOAD_ENDPOINT.to_owned(),
            reason: "未发现可信官方仓库，回退到厂商稳定版下载接口。",
        }
    };

    let candidate = package
        .candidate_version
        .clone()
        .unwrap_or_else(|| "等待解析官方接口".to_owned());
    let update_required = matches!(package.update_state, UpdateState::UpdateAvailable);

    VscodeDetails {
        application_id: "vscode",
        display_name: package.display_name.clone(),
        package_name: package.package_name.clone(),
        installed_version: package.installed_version.clone(),
        candidate_version: package.candidate_version.clone(),
        architecture: package.architecture.clone(),
        support_level: "fullReadOnly",
        trust_state: if trusted {
            TrustState::Trusted
        } else {
            TrustState::NeedsReview
        },
        update_state: package.update_state,
        selected_path,
        fallback_endpoint: STABLE_DOWNLOAD_ENDPOINT,
        evidence: vec![
            TrustEvidence {
                label: "Debian 软件包名",
                actual: package.package_name.clone(),
                expected: PACKAGE_NAME,
                passed: package_matches,
            },
            TrustEvidence {
                label: "系统架构",
                actual: package.architecture.clone(),
                expected: ARCHITECTURE,
                passed: architecture_matches,
            },
            TrustEvidence {
                label: "APT 仓库域名",
                actual: package
                    .source_url
                    .clone()
                    .unwrap_or_else(|| "未发现".to_owned()),
                expected: REPOSITORY_HOST,
                passed: repository_is_official,
            },
        ],
        verification_plan: vec![
            VerificationCheck {
                label: "下载域名",
                expected: "必须属于 Microsoft 允许列表并使用 HTTPS".to_owned(),
                state: CheckState::Planned,
            },
            VerificationCheck {
                label: "包名",
                expected: PACKAGE_NAME.to_owned(),
                state: CheckState::Passed,
            },
            VerificationCheck {
                label: "架构",
                expected: ARCHITECTURE.to_owned(),
                state: CheckState::Passed,
            },
            VerificationCheck {
                label: "版本",
                expected: candidate,
                state: CheckState::Planned,
            },
            VerificationCheck {
                label: "SHA-256",
                expected: "下载完成后与官方索引记录比对".to_owned(),
                state: CheckState::Planned,
            },
        ],
        operation_plan: vec![
            PlanStep {
                order: 1,
                action: "识别来源",
                detail: "确认软件包、架构和 Microsoft 官方仓库".to_owned(),
                state: PlanState::Complete,
            },
            PlanStep {
                order: 2,
                action: "解析候选版本",
                detail: package
                    .candidate_version
                    .as_ref()
                    .map(|version| format!("APT 候选版本 {version}"))
                    .unwrap_or_else(|| "从官方稳定版接口解析".to_owned()),
                state: if package.candidate_version.is_some() {
                    PlanState::Complete
                } else {
                    PlanState::Planned
                },
            },
            PlanStep {
                order: 3,
                action: "下载并校验",
                detail: "下载官方 .deb，并复核域名、包名、架构、版本和 SHA-256".to_owned(),
                state: if update_required {
                    PlanState::Planned
                } else {
                    PlanState::NotRequired
                },
            },
            PlanStep {
                order: 4,
                action: "请求安装授权",
                detail: "仅在校验全部通过后生成特权操作计划".to_owned(),
                state: if update_required {
                    PlanState::Planned
                } else {
                    PlanState::NotRequired
                },
            },
        ],
    }
}

fn url_has_https_host(url: &str, expected_host: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host.eq_ignore_ascii_case(expected_host))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(source_url: Option<&str>, architecture: &str) -> ManagedPackage {
        ManagedPackage {
            package_name: "code".to_owned(),
            display_name: "Visual Studio Code".to_owned(),
            vendor: "Microsoft".to_owned(),
            installed_version: "1.0".to_owned(),
            candidate_version: Some("2.0".to_owned()),
            architecture: architecture.to_owned(),
            source_kind: SourceKind::OfficialRepository,
            source_url: source_url.map(str::to_owned),
            update_state: UpdateState::UpdateAvailable,
            homepage: None,
        }
    }

    #[test]
    fn prefers_the_official_apt_repository() {
        let details = build_details(&package(
            Some("https://packages.microsoft.com/repos/code"),
            "amd64",
        ));
        assert!(matches!(details.trust_state, TrustState::Trusted));
        assert!(matches!(
            details.selected_path.kind,
            UpdatePathKind::OfficialRepository
        ));
        assert!(details.evidence.iter().all(|item| item.passed));
    }

    #[test]
    fn rejects_repository_lookalikes_and_uses_the_stable_endpoint() {
        let details = build_details(&package(
            Some("https://packages.microsoft.com.evil.example/repos/code"),
            "amd64",
        ));
        assert!(matches!(details.trust_state, TrustState::NeedsReview));
        assert!(matches!(
            details.selected_path.kind,
            UpdatePathKind::StableDownloadEndpoint
        ));
    }

    #[test]
    fn requires_amd64_for_the_configured_download_channel() {
        let details = build_details(&package(
            Some("https://packages.microsoft.com/repos/code"),
            "arm64",
        ));
        assert!(matches!(details.trust_state, TrustState::NeedsReview));
        assert!(!details.evidence[1].passed);
    }
}
