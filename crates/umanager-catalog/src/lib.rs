use serde::{Deserialize, Serialize};

/// The single source of truth for every application UManager knows about.
///
/// The JSON file lives at `src-tauri/resources/vendors.json` and is embedded into
/// both the main application and the privileged helper at compile time, so the
/// helper never trusts a user-editable file.
pub const CATALOG_JSON: &str = include_str!("../../../src-tauri/resources/vendors.json");

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub applications: Vec<Application>,
    #[serde(default)]
    pub development_toolchains: Vec<DevelopmentToolchain>,
    /// Single-version command-line developer tools (e.g. AI coding agent CLIs).
    /// These are installed via npm or an official installer, not a version manager.
    #[serde(default)]
    pub development_tools: Vec<DevelopmentTool>,
    /// Optional source that UManager itself checks for self-updates. Kept separate
    /// from `applications` so UManager never shows up in the managed software list.
    #[serde(default)]
    pub self_update: Option<SelfUpdateSource>,
    /// Optional central metadata feed published by the UManager project (e.g. on
    /// GitHub Pages). When configured, UManager prefers this feed for candidate
    /// versions, sizes, SHA-256 digests and download URLs instead of scraping the
    /// vendor websites / APT indexes on the user's machine.
    #[serde(default)]
    pub metadata_feed: Option<MetadataFeed>,
}

/// Where the UManager metadata feed is published and which exact hosts are trusted.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataFeed {
    /// Absolute HTTPS URL of the feed JSON (e.g. GitHub Pages).
    pub url: String,
    /// Exact host names accepted while fetching the feed and following redirects.
    pub hosts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfUpdateSource {
    pub application_id: String,
    pub package_name: String,
    pub display_name: String,
    pub vendor: String,
    pub architecture: String,
    pub release_api_url: String,
    pub release_api_hosts: Vec<String>,
    pub asset_name_pattern: String,
    #[serde(default)]
    pub strip_tag_prefix: Option<String>,
    pub asset_download_hosts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ManagerKind {
    /// Version manager exposed as a shell function that must be sourced (e.g. nvm).
    Shell,
    /// Version manager exposed as an executable on disk (e.g. rustup).
    Binary,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentToolchain {
    pub toolchain_id: String,
    pub display_name: String,
    pub vendor: String,
    pub homepage: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    /// Version-manager identifier, e.g. `nvm`.
    pub manager: String,
    /// How the manager is invoked.
    pub manager_kind: ManagerKind,
    /// Home directory of the manager, e.g. `~/.nvm`; `~` expands to the user home.
    pub manager_home: String,
    /// Script that must be sourced before invoking a `Shell` manager, e.g. `nvm.sh`.
    #[serde(default)]
    pub manager_script: Option<String>,
    /// Executable path for a `Binary` manager, e.g. `~/.cargo/bin/rustup`.
    #[serde(default)]
    pub manager_binary: Option<String>,
    /// Directory that holds one subdirectory per installed version.
    pub versions_directory: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentTool {
    pub tool_id: String,
    pub display_name: String,
    pub vendor: String,
    pub homepage: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    /// Command name of the installed CLI, e.g. `claude`.
    pub binary_name: String,
    /// npm package used for installs, updates, and latest-version lookup.
    pub npm_package: String,
    /// How a missing tool is installed. `npm` runs a global npm install;
    /// `curlScript` runs the vendor's official one-line installer.
    pub installer: DevToolInstaller,
    /// How an officially-installed tool is removed. npm installs always uninstall
    /// through npm regardless of this value.
    pub uninstall: DevToolUninstall,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DevToolInstaller {
    Npm,
    #[serde(rename_all = "camelCase")]
    CurlScript {
        script_url: String,
        /// Exact host serving the install script; recorded for display only.
        host: String,
        /// Shell that executes the piped script (`bash` or `sh`).
        shell: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DevToolUninstall {
    Npm,
    #[serde(rename_all = "camelCase")]
    RemoveFiles { paths: Vec<String> },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    pub application_id: String,
    pub package_name: String,
    pub display_name: String,
    pub vendor: String,
    pub architecture: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Absolute HTTPS icon URL served from the UManager feed (GitHub Pages). When
    /// present, the desktop app prefers this remote icon over a bundled asset.
    #[serde(default)]
    pub icon_url: Option<String>,
    /// SHA-256 (hex) of the icon file at `icon_url`, for integrity + cache naming.
    #[serde(default)]
    pub icon_sha256: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default = "default_true")]
    pub removable: bool,
    pub source: SourceSpec,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SourceSpec {
    #[serde(rename_all = "camelCase")]
    AptRepository {
        /// Exact repository root used to build and validate download URLs.
        repository_url: String,
        /// Hosts accepted while scanning `apt-cache policy` and following redirects.
        repository_hosts: Vec<String>,
        /// Optional fallback download endpoint exposed by the vendor.
        #[serde(default)]
        fallback_download_url: Option<String>,
        /// Optional direct URL of the Debian `Packages` index for this repository.
        /// Used by the CI metadata-feed generator to resolve the candidate version;
        /// the desktop app itself does not read this index.
        #[serde(default)]
        packages_index_url: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    StableDownloadEndpoint {
        /// Vendor page whose HTML exposes the display version.
        official_page_url: String,
        official_page_hosts: Vec<String>,
        /// Fixed .deb download endpoint.
        download_url: String,
        download_hosts: Vec<String>,
        /// HTML marker after which the display version text starts.
        page_version_marker: String,
        /// File name of the download link located on the official page.
        download_link_file_name: String,
        /// Number of dot-separated numeric components in the display version.
        #[serde(default = "default_segments")]
        page_version_segments: usize,
    },
    #[serde(rename_all = "camelCase")]
    ReleaseApi {
        release_api_url: String,
        release_api_hosts: Vec<String>,
        /// Asset name pattern; `{tagVersion}` is replaced with the tag without the
        /// configured prefix (e.g. `FlClash-{tagVersion}-linux-amd64.deb`).
        asset_name_pattern: String,
        /// Prefix stripped from the release tag before matching versions.
        #[serde(default)]
        strip_tag_prefix: Option<String>,
        asset_download_hosts: Vec<String>,
        #[serde(default)]
        allow_prerelease: bool,
    },
    #[serde(rename_all = "camelCase")]
    BrowserImport {
        homepage_url: String,
    },
}

fn default_true() -> bool {
    true
}

fn default_segments() -> usize {
    3
}

impl Catalog {
    pub fn load() -> Result<Self, String> {
        serde_json::from_str(CATALOG_JSON).map_err(|error| format!("内置软件源无效：{error}"))
    }

    pub fn by_application_id(&self, application_id: &str) -> Option<&Application> {
        self.applications
            .iter()
            .find(|application| application.application_id == application_id)
    }

    pub fn by_package_name(&self, package_name: &str) -> Option<&Application> {
        self.applications
            .iter()
            .find(|application| application.package_name == package_name)
    }

    pub fn by_toolchain_id(&self, toolchain_id: &str) -> Option<&DevelopmentToolchain> {
        self.development_toolchains
            .iter()
            .find(|toolchain| toolchain.toolchain_id == toolchain_id)
    }

    pub fn by_tool_id(&self, tool_id: &str) -> Option<&DevelopmentTool> {
        self.development_tools
            .iter()
            .find(|tool| tool.tool_id == tool_id)
    }

    pub fn self_update_source(&self) -> Option<&SelfUpdateSource> {
        self.self_update.as_ref()
    }
}

impl Application {
    /// Whether UManager can resolve, download and plan an install for this entry.
    pub fn is_auto_installable(&self) -> bool {
        matches!(
            self.source,
            SourceSpec::AptRepository { .. }
                | SourceSpec::StableDownloadEndpoint { .. }
                | SourceSpec::ReleaseApi { .. }
        )
    }

    /// Whether the entry is downloaded straight from a vendor website rather than
    /// from a signed APT repository.
    pub fn is_website_download(&self) -> bool {
        matches!(
            self.source,
            SourceSpec::StableDownloadEndpoint { .. } | SourceSpec::ReleaseApi { .. }
        )
    }

    /// Hosts that prove an installed package comes from the vendor's APT repository.
    pub fn apt_repository_hosts(&self) -> Vec<&str> {
        match &self.source {
            SourceSpec::AptRepository {
                repository_hosts, ..
            } => repository_hosts.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }

    /// Exact repository URL, when this entry is an APT repository source.
    pub fn apt_repository_url(&self) -> Option<&str> {
        match &self.source {
            SourceSpec::AptRepository { repository_url, .. } => Some(repository_url),
            _ => None,
        }
    }

    /// Hosts that are allowed to serve the actual `.deb` download for this entry.
    /// This is the host allowlist the download engine still enforces even when the
    /// candidate metadata comes from the central feed.
    pub fn download_hosts(&self) -> Vec<&str> {
        match &self.source {
            SourceSpec::AptRepository { repository_hosts, .. } => {
                repository_hosts.iter().map(String::as_str).collect()
            }
            SourceSpec::StableDownloadEndpoint { download_hosts, .. } => {
                download_hosts.iter().map(String::as_str).collect()
            }
            SourceSpec::ReleaseApi {
                asset_download_hosts, ..
            } => asset_download_hosts.iter().map(String::as_str).collect(),
            SourceSpec::BrowserImport { .. } => Vec::new(),
        }
    }
}

impl SelfUpdateSource {
    /// Renders the self-update source as a regular `releaseApi` application so the
    /// main program can reuse the exact same release fetch / download / verify engine
    /// used for FlClash.
    pub fn to_application(&self) -> Application {
        Application {
            application_id: self.application_id.clone(),
            package_name: self.package_name.clone(),
            display_name: self.display_name.clone(),
            vendor: self.vendor.clone(),
            architecture: self.architecture.clone(),
            homepage: None,
            icon: None,
            icon_url: None,
            icon_sha256: None,
            accent_color: None,
            removable: false,
            source: SourceSpec::ReleaseApi {
                release_api_url: self.release_api_url.clone(),
                release_api_hosts: self.release_api_hosts.clone(),
                asset_name_pattern: self.asset_name_pattern.clone(),
                strip_tag_prefix: self.strip_tag_prefix.clone(),
                asset_download_hosts: self.asset_download_hosts.clone(),
                allow_prerelease: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_valid_and_covers_the_expected_entries() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.applications.len(), 6);
        assert!(catalog.by_package_name("code").unwrap().is_auto_installable());
        assert!(catalog.by_package_name("wechat").unwrap().is_website_download());
        assert!(catalog.by_package_name("flclash").unwrap().is_website_download());
        assert!(!catalog.by_package_name("wemeet").unwrap().is_auto_installable());
        assert!(catalog.applications.iter().all(|item| item.removable));
    }

    #[test]
    fn development_toolchains_are_configuration_driven() {
        let catalog = Catalog::load().unwrap();
        let nodejs = catalog.by_toolchain_id("nodejs").unwrap();
        assert_eq!(nodejs.display_name, "Node.js");
        assert_eq!(nodejs.manager, "nvm");
        assert_eq!(nodejs.manager_kind, ManagerKind::Shell);
        assert!(nodejs.manager_home.contains(".nvm"));
    }

    #[test]
    fn development_tools_are_configuration_driven() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(catalog.development_tools.len(), 4);
        let codex = catalog.by_tool_id("codex").unwrap();
        assert_eq!(codex.binary_name, "codex");
        assert_eq!(codex.npm_package, "@openai/codex");
        assert!(matches!(codex.installer, DevToolInstaller::Npm));
        let claude = catalog.by_tool_id("claude-code").unwrap();
        assert!(matches!(claude.installer, DevToolInstaller::CurlScript { .. }));
        assert!(matches!(claude.uninstall, DevToolUninstall::RemoveFiles { .. }));
    }

    #[test]
    fn rust_toolchain_is_configured_as_a_binary_manager() {
        let catalog = Catalog::load().unwrap();
        let rust = catalog.by_toolchain_id("rust").unwrap();
        assert_eq!(rust.manager, "rustup");
        assert_eq!(rust.manager_kind, ManagerKind::Binary);
        assert!(rust.versions_directory.contains(".rustup"));
        assert!(rust.manager_binary.as_deref().is_some_and(|path| path.contains("rustup")));
    }

    #[test]
    fn every_auto_installable_entry_has_a_unique_application_id() {
        let catalog = Catalog::load().unwrap();
        let mut ids: Vec<_> = catalog
            .applications
            .iter()
            .filter(|item| item.is_auto_installable())
            .map(|item| item.application_id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn self_update_source_is_configured_as_a_release_api() {
        let catalog = Catalog::load().unwrap();
        let source = catalog.self_update_source().expect("self-update source");
        assert_eq!(source.package_name, "u-manager");
        assert_eq!(source.architecture, "amd64");
        let application = source.to_application();
        assert!(application.is_website_download());
        assert!(!application.removable);
        assert!(matches!(application.source, SourceSpec::ReleaseApi { .. }));
    }
}
