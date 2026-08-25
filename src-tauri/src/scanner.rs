use serde::Serialize;
use std::collections::HashMap;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};
use umanager_catalog::Catalog;

const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const DPKG_FORMAT: &str =
    "${binary:Package}\t${Version}\t${Architecture}\t${Status}\t${Homepage}\n";

#[derive(Clone, Debug, PartialEq, Eq)]
struct InstalledPackage {
    package_name: String,
    version: String,
    architecture: String,
    homepage: Option<String>,
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

/// Lists which managed applications are installed locally. Candidate versions,
/// update state and the official source URL are no longer resolved here — the
/// caller fills them from the central metadata feed.
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

    for package in installed {
        let Some(application) = definitions.get(package.package_name.as_str()) else {
            continue;
        };
        packages.push(ManagedPackage {
            package_name: package.package_name,
            display_name: application.display_name.clone(),
            vendor: application.vendor.clone(),
            installed_version: package.version,
            candidate_version: None,
            architecture: package.architecture,
            source_kind: SourceKind::LocalPackage,
            source_url: None,
            update_state: UpdateState::Unknown,
            homepage: application.homepage.clone().or(package.homepage),
        });
    }

    packages.sort_by(|left, right| left.display_name.cmp(&right.display_name));

    Ok(ScanResult {
        packages,
        scanned_at_unix_seconds: unix_timestamp_now(),
        warnings: Vec::new(),
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
        assert!(catalog.metadata_feed.is_some());
        assert!(catalog
            .by_application_id("vscode")
            .and_then(|app| match &app.source {
                umanager_catalog::SourceSpec::AptRepository {
                    packages_index_url,
                    ..
                } => packages_index_url.as_ref(),
                _ => None,
            })
            .is_some());
    }

    #[test]
    fn system_commands_use_a_locale_independent_output_format() {
        let command = locale_stable_command(DPKG_QUERY_BIN);
        let environment: HashMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|item| item.to_string_lossy().into_owned()),
                )
            })
            .collect();

        assert_eq!(command.get_program(), DPKG_QUERY_BIN);
        assert_eq!(environment.get("LC_ALL"), Some(&Some("C".to_owned())));
        assert_eq!(environment.get("LANG"), Some(&Some("C".to_owned())));
        assert_eq!(environment.get("LANGUAGE"), Some(&Some("C".to_owned())));
    }
}
