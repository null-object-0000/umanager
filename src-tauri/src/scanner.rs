use serde::Serialize;
use std::collections::HashMap;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};
use umanager_catalog::{Application, Catalog, SourceSpec};

const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const APT_CACHE_BIN: &str = "/usr/bin/apt-cache";
const DPKG_BIN: &str = "/usr/bin/dpkg";
const DPKG_FORMAT: &str =
    "${binary:Package}\t${Version}\t${Architecture}\t${Status}\t${Homepage}\n";

#[derive(Clone, Debug, PartialEq, Eq)]
struct InstalledPackage {
    package_name: String,
    version: String,
    architecture: String,
    homepage: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AptPolicy {
    pub(crate) candidate: Option<String>,
    pub(crate) repository_urls: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPackage {
    pub(crate) package_name: String,
    pub(crate) display_name: String,
    pub(crate) vendor: String,
    pub(crate) installed_version: String,
    pub(crate) candidate_version: Option<String>,
    pub(crate) architecture: String,
    pub(crate) source_kind: SourceKind,
    pub(crate) source_url: Option<String>,
    pub(crate) update_state: UpdateState,
    pub(crate) homepage: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    OfficialRepository,
    OfficialWebsite,
    LocalPackage,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateState {
    UpToDate,
    UpdateAvailable,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub(crate) packages: Vec<ManagedPackage>,
    scanned_at_unix_seconds: u64,
    pub(crate) warnings: Vec<String>,
}

pub fn scan() -> Result<ScanResult, String> {
    let catalog = Catalog::load()?;
    let output = locale_stable_command(DPKG_QUERY_BIN)
        .args(["-W", "-f", DPKG_FORMAT])
        .output()
        .map_err(|error| format!("无法执行 dpkg-query：{error}"))?;

    if !output.status.success() {
        return Err(command_error("dpkg-query", &output));
    }

    let installed = parse_dpkg_query(&String::from_utf8_lossy(&output.stdout));
    let definitions: HashMap<_, _> = catalog
        .applications
        .iter()
        .map(|application| (application.package_name.as_str(), application))
        .collect();
    let mut packages = Vec::new();
    let mut warnings = Vec::new();

    for package in installed {
        let Some(application) = definitions.get(package.package_name.as_str()) else {
            continue;
        };

        match inspect_package(package, application) {
            Ok(item) => {
                if matches!(item.source_kind, SourceKind::OfficialRepository)
                    && item.candidate_version.is_none()
                {
                    warnings.push(format!(
                        "{} 已连接官方仓库，但未能从本机 APT 缓存解析候选版本。可尝试刷新软件包索引后重新扫描。",
                        item.display_name
                    ));
                }
                packages.push(item);
            }
            Err(warning) => warnings.push(warning),
        }
    }

    packages.sort_by(|left, right| left.display_name.cmp(&right.display_name));

    Ok(ScanResult {
        packages,
        scanned_at_unix_seconds: unix_timestamp_now(),
        warnings,
    })
}

fn inspect_package(
    installed: InstalledPackage,
    application: &Application,
) -> Result<ManagedPackage, String> {
    let SourceSpec::AptRepository { .. } = application.source else {
        // Website and browser-import sources are not resolved from the APT index here;
        // website candidates are attached later by the source engine.
        return Ok(ManagedPackage {
            package_name: installed.package_name,
            display_name: application.display_name.clone(),
            vendor: application.vendor.clone(),
            installed_version: installed.version,
            candidate_version: None,
            architecture: installed.architecture,
            source_kind: SourceKind::LocalPackage,
            source_url: None,
            update_state: UpdateState::Unknown,
            homepage: application.homepage.clone().or(installed.homepage),
        });
    };

    let output = locale_stable_command(APT_CACHE_BIN)
        .args(["policy", &installed.package_name])
        .output()
        .map_err(|error| format!("无法检查 {} 的 APT 缓存：{error}", installed.package_name))?;

    if !output.status.success() {
        return Err(command_error(
            &format!("apt-cache policy {}", installed.package_name),
            &output,
        ));
    }

    let policy = parse_apt_policy(&String::from_utf8_lossy(&output.stdout));
    let official_url = policy
        .repository_urls
        .iter()
        .find(|url| {
            application
                .apt_repository_hosts()
                .iter()
                .any(|host| url_has_host(url, host))
        });
    let has_official_repository = official_url.is_some();
    let candidate = has_official_repository
        .then_some(policy.candidate)
        .flatten();
    let update_state = if has_official_repository {
        match candidate.as_deref() {
            Some(version) if version_is_newer(&installed.version, version) => {
                UpdateState::UpdateAvailable
            }
            Some(_) => UpdateState::UpToDate,
            None => UpdateState::Unknown,
        }
    } else {
        UpdateState::Unknown
    };

    Ok(ManagedPackage {
        package_name: installed.package_name,
        display_name: application.display_name.clone(),
        vendor: application.vendor.clone(),
        installed_version: installed.version,
        candidate_version: candidate,
        architecture: installed.architecture,
        source_kind: if has_official_repository {
            SourceKind::OfficialRepository
        } else {
            SourceKind::LocalPackage
        },
        source_url: official_url.cloned(),
        update_state,
        homepage: application.homepage.clone().or(installed.homepage),
    })
}

fn parse_dpkg_query(input: &str) -> Vec<InstalledPackage> {
    input
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(5, '\t');
            let package_name = fields.next()?.split(':').next()?.trim();
            let version = fields.next()?.trim();
            let architecture = fields.next()?.trim();
            let status = fields.next()?.trim();
            let homepage = fields
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            (status == "install ok installed").then(|| InstalledPackage {
                package_name: package_name.to_owned(),
                version: version.to_owned(),
                architecture: architecture.to_owned(),
                homepage: homepage.map(str::to_owned),
            })
        })
        .collect()
}

pub(crate) fn parse_apt_policy(input: &str) -> AptPolicy {
    let candidate = input.lines().find_map(|line| {
        line.trim()
            .strip_prefix("Candidate:")
            .map(str::trim)
            .filter(|value| *value != "(none)")
            .map(str::to_owned)
    });
    let repository_urls = input
        .lines()
        .filter_map(|line| {
            line.split_whitespace()
                .find(|field| field.starts_with("https://") || field.starts_with("http://"))
                .map(|url| url.trim_end_matches('/').to_owned())
        })
        .fold(Vec::new(), |mut urls, url| {
            if !urls.contains(&url) {
                urls.push(url);
            }
            urls
        });

    AptPolicy {
        candidate,
        repository_urls,
    }
}

fn url_has_host(url: &str, expected_host: &str) -> bool {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    without_scheme
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host.eq_ignore_ascii_case(expected_host))
}

fn version_is_newer(installed: &str, candidate: &str) -> bool {
    locale_stable_command(DPKG_BIN)
        .args(["--compare-versions", installed, "lt", candidate])
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn locale_stable_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    command
}

fn command_error(command: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{command} 执行失败：{}", stderr.trim())
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_installed_packages_and_ignores_other_states() {
        let input = concat!(
            "code\t1.134.0-1\tamd64\tinstall ok installed\thttps://code.visualstudio.com/\n",
            "old\t1.0\tamd64\tdeinstall ok config-files\t\n",
        );
        assert_eq!(
            parse_dpkg_query(input),
            vec![InstalledPackage {
                package_name: "code".into(),
                version: "1.134.0-1".into(),
                architecture: "amd64".into(),
                homepage: Some("https://code.visualstudio.com/".into()),
            }]
        );
    }

    #[test]
    fn parses_candidate_and_deduplicates_repository_urls() {
        let input = r#"code:
  Installed: 1.0
  Candidate: 2.0
  Version table:
     2.0 500
        500 https://packages.microsoft.com/repos/code stable/main amd64 Packages
 *** 1.0 100
        100 /var/lib/dpkg/status
"#;
        assert_eq!(
            parse_apt_policy(input),
            AptPolicy {
                candidate: Some("2.0".into()),
                repository_urls: vec!["https://packages.microsoft.com/repos/code".into()],
            }
        );
    }

    #[test]
    fn host_matching_does_not_accept_lookalike_domains() {
        assert!(url_has_host(
            "https://packages.microsoft.com/repos/code",
            "packages.microsoft.com"
        ));
        assert!(!url_has_host(
            "https://packages.microsoft.com.evil.example/repos/code",
            "packages.microsoft.com"
        ));
    }

    #[test]
    fn embedded_catalog_is_valid_and_covers_the_supported_applications() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.applications.len(), 6);
        assert!(catalog
            .applications
            .iter()
            .all(|item| !item.package_name.is_empty() && !item.display_name.is_empty()));
        assert!(catalog.by_application_id("vscode").is_some());
        assert!(catalog.by_application_id("wechat").is_some());
        assert!(catalog.by_application_id("flclash").is_some());
        assert!(catalog.by_application_id("wemeet").is_some());
    }

    #[test]
    fn system_commands_use_a_locale_independent_output_format() {
        let command = locale_stable_command(APT_CACHE_BIN);
        let environment: HashMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|item| item.to_string_lossy().into_owned()),
                )
            })
            .collect();

        assert_eq!(command.get_program(), APT_CACHE_BIN);
        assert_eq!(environment.get("LC_ALL"), Some(&Some("C".to_owned())));
        assert_eq!(environment.get("LANG"), Some(&Some("C".to_owned())));
        assert_eq!(environment.get("LANGUAGE"), Some(&Some("C".to_owned())));
    }
}
