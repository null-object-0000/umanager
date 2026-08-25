use crate::scanner::{self, SourceKind, UpdateState};
use reqwest::header::{CONTENT_RANGE, RANGE};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use umanager_catalog::{Application, SourceSpec};

const APT_CACHE_BIN: &str = "/usr/bin/apt-cache";
const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";
const DPKG_BIN: &str = "/usr/bin/dpkg";
const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
const CONTROL_PROBE_BYTES: u64 = 4 * 1024;
const MAX_CONTROL_PREFIX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_JSON_BYTES: u64 = 1024 * 1024;
const REMOTE_CACHE_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDetails {
    pub application_id: String,
    pub display_name: String,
    pub package_name: String,
    pub vendor: String,
    pub architecture: String,
    pub source_kind: SourceKind,
    pub source_url: String,
    pub installed_version: Option<String>,
    pub candidate_version: Option<String>,
    pub update_state: UpdateState,
    pub website_version: Option<String>,
    pub expected_size: Option<u64>,
    pub sha256: Option<String>,
    pub metadata_bytes: Option<u64>,
    pub release_tag: Option<String>,
    pub asset_name: Option<String>,
    pub trusted: bool,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub label: String,
    pub actual: String,
    pub expected: String,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPlan {
    pub application_id: String,
    pub package_name: String,
    pub version: String,
    pub architecture: String,
    pub source_kind: SourceKind,
    pub repository_url: Option<String>,
    pub download_url: String,
    pub file_name: String,
    pub expected_size: u64,
    pub expected_sha256: Option<String>,
    pub target_path: String,
    pub release_tag: Option<String>,
    pub asset_name: Option<String>,
    pub website_version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    pub plan: DownloadPlan,
    pub actual_size: u64,
    pub actual_sha256: String,
    pub package_name: String,
    pub version: String,
    pub architecture: String,
    pub reused_existing_file: bool,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub package_name: String,
    pub phase: &'static str,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
}

pub type ProgressCallback = Arc<dyn Fn(DownloadProgress) + Send + Sync>;

pub(crate) struct Installable {
    pub(crate) installed_version: Option<String>,
    pub(crate) candidate_version: Option<String>,
    pub(crate) download_plan: Option<DownloadPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PackageIndexRecord {
    package: String,
    version: String,
    architecture: String,
    filename: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug)]
struct DebMetadata {
    package: String,
    version: String,
    architecture: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PageMetadata {
    version: String,
    download_url: String,
}

#[derive(Clone, Debug)]
struct ReleaseMetadata {
    tag_name: String,
    tag_version: String,
    asset_name: String,
    download_url: String,
    expected_size: u64,
    sha256: String,
}

#[derive(Clone, Debug)]
struct PackageMetadata {
    package_name: String,
    version: String,
    architecture: String,
    total_size: u64,
    metadata_bytes: u64,
}

#[derive(Clone, Debug)]
struct WebsiteRemote {
    display_version: String,
    download_url: String,
    expected_size: u64,
    expected_sha256: Option<String>,
    release_tag: Option<String>,
    asset_name: Option<String>,
    package: PackageMetadata,
}

#[derive(Clone, Debug)]
struct CachedRemoteMetadata {
    fetched_at: Instant,
    metadata: WebsiteRemote,
}

#[derive(Debug)]
struct RangePayload {
    bytes: Vec<u8>,
    total_size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    end: u64,
    total: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteKind {
    StableDownload,
    ReleaseApi,
}

static REMOTE_CACHE: OnceLock<Mutex<HashMap<String, CachedRemoteMetadata>>> = OnceLock::new();

pub async fn load_details(app: &Application, cache_dir: &Path) -> Result<ApplicationDetails, String> {
    match &app.source {
        SourceSpec::AptRepository { .. } => load_apt_details(app),
        SourceSpec::StableDownloadEndpoint { .. } | SourceSpec::ReleaseApi { .. } => {
            load_website_details(app, cache_dir).await
        }
        SourceSpec::BrowserImport { .. } => {
            Err(format!("{} 使用浏览器导入，不支持自动更新检查", app.display_name))
        }
    }
}

fn load_apt_details(app: &Application) -> Result<ApplicationDetails, String> {
    let installed = installed_package_version_optional(app)?;
    let policy = load_apt_policy(app)?;
    let matched_repo = policy
        .repository_urls
        .iter()
        .find(|url| apt_repository_matches(url, app))
        .cloned();
    let candidate = matched_repo
        .is_some()
        .then_some(policy.candidate)
        .flatten();
    let update_state = match (installed.as_deref(), candidate.as_deref()) {
        (Some(installed), Some(candidate)) if debian_version_is_newer(installed, candidate) => {
            UpdateState::UpdateAvailable
        }
        (Some(_), Some(_)) => UpdateState::UpToDate,
        _ => UpdateState::Unknown,
    };
    let source_url = matched_repo
        .clone()
        .or_else(|| app.apt_repository_url().map(str::to_owned))
        .unwrap_or_default();
    let expected_repo = app.apt_repository_url().unwrap_or_default().to_owned();
    let actual_repo = matched_repo.clone().unwrap_or_else(|| "未发现".to_owned());
    let trusted = matched_repo.is_some();

    Ok(ApplicationDetails {
        application_id: app.application_id.clone(),
        display_name: app.display_name.clone(),
        package_name: app.package_name.clone(),
        vendor: app.vendor.clone(),
        architecture: app.architecture.clone(),
        source_kind: SourceKind::OfficialRepository,
        source_url,
        installed_version: installed,
        candidate_version: candidate,
        update_state,
        website_version: None,
        expected_size: None,
        sha256: None,
        metadata_bytes: None,
        release_tag: None,
        asset_name: None,
        trusted,
        evidence: vec![
            Evidence {
                label: "Debian 软件包名".to_owned(),
                actual: app.package_name.clone(),
                expected: app.package_name.clone(),
                passed: true,
            },
            Evidence {
                label: "系统架构".to_owned(),
                actual: app.architecture.clone(),
                expected: app.architecture.clone(),
                passed: true,
            },
            Evidence {
                label: "APT 仓库域名".to_owned(),
                actual: actual_repo,
                expected: expected_repo,
                passed: trusted,
            },
        ],
    })
}

async fn load_website_details(
    app: &Application,
    cache_dir: &Path,
) -> Result<ApplicationDetails, String> {
    let installed = installed_package_version_optional(app)?;
    let remote = website_remote(app, cache_dir).await?;
    let update_state = match installed.as_deref() {
        Some(installed) if debian_version_is_newer(installed, &remote.package.version) => {
            UpdateState::UpdateAvailable
        }
        Some(_) => UpdateState::UpToDate,
        None => UpdateState::Unknown,
    };
    let kind = remote_kind(app);
    let evidence = match kind {
        RemoteKind::StableDownload => {
            let (page_hosts, download_hosts) = stable_hosts(app)?;
            vec![
                Evidence {
                    label: "官网域名".to_owned(),
                    actual: page_hosts.join(", "),
                    expected: page_hosts.join(", "),
                    passed: true,
                },
                Evidence {
                    label: "下载域名".to_owned(),
                    actual: download_hosts.join(", "),
                    expected: download_hosts.join(", "),
                    passed: true,
                },
                Evidence {
                    label: "Debian 软件包名".to_owned(),
                    actual: remote.package.package_name.clone(),
                    expected: app.package_name.clone(),
                    passed: true,
                },
                Evidence {
                    label: "软件包架构".to_owned(),
                    actual: remote.package.architecture.clone(),
                    expected: app.architecture.clone(),
                    passed: true,
                },
            ]
        }
        RemoteKind::ReleaseApi => {
            let (api_hosts, download_hosts) = release_hosts(app)?;
            vec![
                Evidence {
                    label: "发布 API 域名".to_owned(),
                    actual: api_hosts.join(", "),
                    expected: api_hosts.join(", "),
                    passed: true,
                },
                Evidence {
                    label: "发布资产域名".to_owned(),
                    actual: download_hosts.join(", "),
                    expected: download_hosts.join(", "),
                    passed: true,
                },
                Evidence {
                    label: "Debian 软件包名".to_owned(),
                    actual: remote.package.package_name.clone(),
                    expected: app.package_name.clone(),
                    passed: true,
                },
                Evidence {
                    label: "软件包架构".to_owned(),
                    actual: remote.package.architecture.clone(),
                    expected: app.architecture.clone(),
                    passed: true,
                },
            ]
        }
    };

    Ok(ApplicationDetails {
        application_id: app.application_id.clone(),
        display_name: app.display_name.clone(),
        package_name: app.package_name.clone(),
        vendor: app.vendor.clone(),
        architecture: app.architecture.clone(),
        source_kind: SourceKind::OfficialWebsite,
        source_url: remote.download_url.clone(),
        installed_version: installed,
        candidate_version: Some(remote.package.version.clone()),
        update_state,
        website_version: Some(remote.display_version.clone()),
        expected_size: Some(remote.expected_size),
        sha256: remote.expected_sha256.clone(),
        metadata_bytes: Some(remote.package.metadata_bytes),
        release_tag: remote.release_tag.clone(),
        asset_name: remote.asset_name.clone(),
        trusted: true,
        evidence,
    })
}

pub async fn build_download_plan(app: &Application, cache_dir: &Path) -> Result<DownloadPlan, String> {
    match &app.source {
        SourceSpec::AptRepository { .. } => apt_build_plan(app, cache_dir),
        SourceSpec::StableDownloadEndpoint { .. } | SourceSpec::ReleaseApi { .. } => {
            let remote = website_remote(app, cache_dir).await?;
            website_build_plan(app, cache_dir, &remote)
        }
        SourceSpec::BrowserImport { .. } => {
            Err(format!("{} 使用浏览器导入，无法生成下载计划", app.display_name))
        }
    }
}

pub(crate) async fn load_installable(app: &Application, cache_dir: &Path) -> Result<Installable, String> {
    match &app.source {
        SourceSpec::AptRepository { .. } => {
            let installed = installed_package_version_optional(app)?;
            let policy = load_apt_policy(app)?;
            let repository_configured = policy
                .repository_urls
                .iter()
                .any(|url| apt_repository_matches(url, app));
            if installed.is_some() {
                return Ok(Installable {
                    installed_version: installed,
                    candidate_version: policy.candidate,
                    download_plan: None,
                });
            }
            let plan = if repository_configured && policy.candidate.is_some() {
                Some(apt_build_plan(app, cache_dir)?)
            } else {
                None
            };
            Ok(Installable {
                installed_version: None,
                candidate_version: policy.candidate,
                download_plan: plan,
            })
        }
        SourceSpec::StableDownloadEndpoint { .. } | SourceSpec::ReleaseApi { .. } => {
            let installed = installed_package_version_optional(app)?;
            if installed.is_some() {
                return Ok(Installable {
                    installed_version: installed,
                    candidate_version: None,
                    download_plan: None,
                });
            }
            let remote = website_remote(app, cache_dir).await?;
            let plan = website_build_plan(app, cache_dir, &remote)?;
            Ok(Installable {
                installed_version: None,
                candidate_version: Some(remote.package.version),
                download_plan: Some(plan),
            })
        }
        SourceSpec::BrowserImport { .. } => Ok(Installable {
            installed_version: installed_package_version_optional(app)?,
            candidate_version: None,
            download_plan: None,
        }),
    }
}

pub async fn download_and_verify(
    app: &Application,
    cache_dir: PathBuf,
    progress: ProgressCallback,
) -> Result<DownloadResult, String> {
    let plan = build_download_plan(app, &cache_dir).await?;
    let target_path = PathBuf::from(&plan.target_path);
    let parent = target_path
        .parent()
        .ok_or_else(|| "下载缓存路径无效".to_owned())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("无法创建下载缓存目录：{error}"))?;
    if target_path.is_file() {
        let verified = verify_plan_file(app, &plan, &target_path, Some(&progress)).await?;
        emit_progress(&progress, &plan, "completed", verified.0, 0);
        return Ok(result_from_verified(plan, verified, true));
    }

    let temporary_path = parent.join(format!(
        ".{}-{}-{}.tmp",
        plan.package_name,
        std::process::id(),
        unix_timestamp()
    ));
    let verified = match download_plan_file(app, &plan, &temporary_path, &progress).await {
        Ok(value) => value,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(error);
        }
    };
    if let Err(error) = tokio::fs::hard_link(&temporary_path, &target_path).await {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(format!("无法将已校验安装包写入缓存：{error}"));
    }
    tokio::fs::remove_file(&temporary_path)
        .await
        .map_err(|error| format!("已写入缓存，但无法清理临时下载文件：{error}"))?;
    emit_progress(&progress, &plan, "completed", verified.0, 0);
    Ok(result_from_verified(plan, verified, false))
}

pub async fn verify_cached(app: &Application, cache_dir: &Path) -> Result<DownloadResult, String> {
    let plan = build_download_plan(app, cache_dir).await?;
    let target_path = PathBuf::from(&plan.target_path);
    if !target_path.is_file() {
        return Err("安装包尚未下载或不再位于缓存中".to_owned());
    }
    let verified = verify_plan_file(app, &plan, &target_path, None).await?;
    Ok(result_from_verified(plan, verified, true))
}

fn is_apt_source(app: &Application) -> bool {
    matches!(app.source, SourceSpec::AptRepository { .. })
}

async fn download_plan_file(
    app: &Application,
    plan: &DownloadPlan,
    path: &Path,
    progress: &ProgressCallback,
) -> Result<VerifiedFile, String> {
    if is_apt_source(app) {
        apt_download_to_file(app, plan, path, progress).await
    } else {
        download_website_to_file(app, plan, path, progress).await
    }
}

async fn verify_plan_file(
    app: &Application,
    plan: &DownloadPlan,
    path: &Path,
    progress: Option<&ProgressCallback>,
) -> Result<VerifiedFile, String> {
    if is_apt_source(app) {
        apt_verify_file(app, plan, path, progress).await
    } else {
        verify_website_file(app, plan, path, progress).await
    }
}

type VerifiedFile = (u64, String, DebMetadata);

fn result_from_verified(
    plan: DownloadPlan,
    verified: VerifiedFile,
    reused_existing_file: bool,
) -> DownloadResult {
    DownloadResult {
        package_name: verified.2.package,
        version: verified.2.version,
        architecture: verified.2.architecture,
        actual_size: verified.0,
        actual_sha256: verified.1,
        plan,
        reused_existing_file,
        verified: true,
    }
}

// ---------------------------------------------------------------------------
// APT repository source
// ---------------------------------------------------------------------------

fn apt_build_plan(app: &Application, cache_dir: &Path) -> Result<DownloadPlan, String> {
    let policy = load_apt_policy(app)?;
    let version = policy
        .candidate
        .as_deref()
        .ok_or_else(|| format!("APT 缓存中没有 {} 候选版本", app.display_name))?;
    let repository_url = policy
        .repository_urls
        .iter()
        .map(String::as_str)
        .find(|url| apt_repository_matches(url, app))
        .ok_or_else(|| format!("未发现可信的 {} 官方 APT 仓库", app.display_name))?;

    let output = scanner::locale_stable_command(APT_CACHE_BIN)
        .arg("show")
        .arg(format!("{}={version}", app.package_name))
        .output()
        .map_err(|error| format!("无法读取 {} APT 包索引：{error}", app.display_name))?;
    if !output.status.success() {
        return Err(format!(
            "读取 {} APT 包索引失败：{}",
            app.display_name,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let record = parse_package_index(&String::from_utf8_lossy(&output.stdout), version, app)?;
    validate_index_record(&record, version, app)?;
    let file_name = safe_deb_file_name(&record.filename)?;
    let download_url = join_repository_url(repository_url, &record.filename, app)?;
    let target_path = cache_dir.join("downloads").join(&file_name);

    Ok(DownloadPlan {
        application_id: app.application_id.clone(),
        package_name: record.package,
        version: record.version,
        architecture: record.architecture,
        source_kind: SourceKind::OfficialRepository,
        repository_url: Some(repository_url.to_owned()),
        download_url,
        file_name,
        expected_size: record.size,
        expected_sha256: Some(record.sha256),
        target_path: target_path.to_string_lossy().into_owned(),
        release_tag: None,
        asset_name: None,
        website_version: None,
    })
}

fn load_apt_policy(app: &Application) -> Result<scanner::AptPolicy, String> {
    let output = scanner::locale_stable_command(APT_CACHE_BIN)
        .args(["policy", &app.package_name])
        .output()
        .map_err(|error| format!("无法读取 {} APT 策略：{error}", app.display_name))?;
    if !output.status.success() {
        return Err(format!(
            "读取 {} APT 策略失败：{}",
            app.display_name,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(scanner::parse_apt_policy(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn apt_repository_matches(url: &str, app: &Application) -> bool {
    url.trim_end_matches('/').eq_ignore_ascii_case(
        app.apt_repository_url()
            .unwrap_or_default()
            .trim_end_matches('/'),
    )
}

fn parse_package_index(
    input: &str,
    expected_version: &str,
    app: &Application,
) -> Result<PackageIndexRecord, String> {
    for paragraph in input.split("\n\n") {
        let fields: HashMap<_, _> = paragraph
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim(), value.trim()))
            .collect();
        if fields.get("Package") == Some(&app.package_name.as_str())
            && fields.get("Version") == Some(&expected_version)
            && fields.get("Architecture") == Some(&app.architecture.as_str())
        {
            return Ok(PackageIndexRecord {
                package: required_field(&fields, "Package")?.to_owned(),
                version: required_field(&fields, "Version")?.to_owned(),
                architecture: required_field(&fields, "Architecture")?.to_owned(),
                filename: required_field(&fields, "Filename")?.to_owned(),
                size: required_field(&fields, "Size")?
                    .parse()
                    .map_err(|_| "APT 索引中的 Size 无效".to_owned())?,
                sha256: required_field(&fields, "SHA256")?.to_ascii_lowercase(),
            });
        }
    }
    Err(format!(
        "APT 包索引中没有找到 {} {expected_version} {}",
        app.package_name, app.architecture
    ))
}

fn required_field<'a>(fields: &'a HashMap<&str, &str>, name: &str) -> Result<&'a str, String> {
    fields
        .get(name)
        .copied()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("APT 包索引缺少字段 {name}"))
}

fn validate_index_record(
    record: &PackageIndexRecord,
    version: &str,
    app: &Application,
) -> Result<(), String> {
    if record.package != app.package_name
        || record.architecture != app.architecture
        || record.version != version
    {
        return Err("APT 包索引与官方仓库下载计划不匹配".to_owned());
    }
    if record.size == 0 {
        return Err("APT 包索引声明的文件大小为零".to_owned());
    }
    if record.sha256.len() != 64 || !record.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("APT 包索引中的 SHA256 无效".to_owned());
    }
    Ok(())
}

fn safe_deb_file_name(filename: &str) -> Result<String, String> {
    let path = Path::new(filename);
    if path.is_absolute() || filename.split('/').any(|part| part == "..") {
        return Err("APT 包索引包含不安全的文件路径".to_owned());
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| value.ends_with(".deb") && !value.is_empty())
        .ok_or_else(|| "APT 包索引中的文件名不是有效的 .deb".to_owned())?;
    Ok(file_name.to_owned())
}

fn join_repository_url(repository: &str, filename: &str, app: &Application) -> Result<String, String> {
    if !apt_repository_matches(repository, app) {
        return Err("APT 仓库 URL 不在该应用的允许列表中".to_owned());
    }
    safe_deb_file_name(filename)?;
    Ok(format!(
        "{}/{}",
        repository.trim_end_matches('/'),
        filename.trim_start_matches('/')
    ))
}

// ---------------------------------------------------------------------------
// Website sources (stable download endpoint + GitHub releases API)
// ---------------------------------------------------------------------------

fn remote_kind(app: &Application) -> RemoteKind {
    match &app.source {
        SourceSpec::StableDownloadEndpoint { .. } => RemoteKind::StableDownload,
        SourceSpec::ReleaseApi { .. } => RemoteKind::ReleaseApi,
        _ => unreachable!("only website sources reach remote_kind"),
    }
}

fn stable_hosts(app: &Application) -> Result<(Vec<String>, Vec<String>), String> {
    match &app.source {
        SourceSpec::StableDownloadEndpoint {
            official_page_hosts,
            download_hosts,
            ..
        } => Ok((official_page_hosts.clone(), download_hosts.clone())),
        _ => Err("该应用不是固定下载地址来源".to_owned()),
    }
}

fn release_hosts(app: &Application) -> Result<(Vec<String>, Vec<String>), String> {
    match &app.source {
        SourceSpec::ReleaseApi {
            release_api_hosts,
            asset_download_hosts,
            ..
        } => Ok((release_api_hosts.clone(), asset_download_hosts.clone())),
        _ => Err("该应用不是发布 API 来源".to_owned()),
    }
}

fn website_download_hosts(app: &Application) -> Result<Vec<String>, String> {
    match &app.source {
        SourceSpec::StableDownloadEndpoint { download_hosts, .. } => Ok(download_hosts.clone()),
        SourceSpec::ReleaseApi {
            asset_download_hosts,
            ..
        } => Ok(asset_download_hosts.clone()),
        _ => Err("该应用不是官网下载来源".to_owned()),
    }
}

async fn website_remote(app: &Application, cache_dir: &Path) -> Result<WebsiteRemote, String> {
    let cache = REMOTE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().await;
        if let Some(entry) = guard.get(&app.application_id)
            && entry.fetched_at.elapsed() < REMOTE_CACHE_TTL
        {
            return Ok(entry.metadata.clone());
        }
    }

    let package_cache_dir = cache_dir.to_path_buf();
    let remote = match remote_kind(app) {
        RemoteKind::StableDownload => {
            let page = fetch_page_metadata(app).await?;
            let package =
                fetch_package_metadata_prefix(app, &package_cache_dir, &page.download_url).await?;
            WebsiteRemote {
                display_version: page.version,
                download_url: page.download_url,
                expected_size: package.total_size,
                expected_sha256: None,
                release_tag: None,
                asset_name: None,
                package,
            }
        }
        RemoteKind::ReleaseApi => {
            let release = fetch_release_metadata(app).await?;
            let package = fetch_package_metadata_prefix(
                app,
                &package_cache_dir,
                &release.download_url,
            )
            .await?;
            WebsiteRemote {
                display_version: release.tag_version,
                download_url: release.download_url,
                expected_size: release.expected_size,
                expected_sha256: Some(release.sha256),
                release_tag: Some(release.tag_name),
                asset_name: Some(release.asset_name),
                package,
            }
        }
    };
    validate_website_remote(app, &remote)?;
    let mut guard = cache.lock().await;
    guard.insert(
        app.application_id.clone(),
        CachedRemoteMetadata {
            fetched_at: Instant::now(),
            metadata: remote.clone(),
        },
    );
    Ok(remote)
}

fn validate_website_remote(app: &Application, remote: &WebsiteRemote) -> Result<(), String> {
    if remote.package.package_name != app.package_name
        || remote.package.architecture != app.architecture
        || remote.package.total_size != remote.expected_size
        || !website_version_matches(app, &remote.package.version, &remote.display_version)
    {
        return Err(format!(
            "{} 官网证据、包名、版本或架构不符合软件源策略",
            app.display_name
        ));
    }
    Ok(())
}

fn website_build_plan(
    app: &Application,
    cache_dir: &Path,
    remote: &WebsiteRemote,
) -> Result<DownloadPlan, String> {
    let version_hash = format!("{:x}", Sha256::digest(remote.package.version.as_bytes()));
    let file_name = format!("{}-{}.deb", app.package_name, &version_hash[..16]);
    Ok(DownloadPlan {
        application_id: app.application_id.clone(),
        package_name: remote.package.package_name.clone(),
        version: remote.package.version.clone(),
        architecture: remote.package.architecture.clone(),
        source_kind: SourceKind::OfficialWebsite,
        repository_url: None,
        download_url: remote.download_url.clone(),
        file_name: file_name.clone(),
        expected_size: remote.expected_size,
        expected_sha256: remote.expected_sha256.clone(),
        target_path: cache_dir
            .join("downloads")
            .join(file_name)
            .to_string_lossy()
            .into_owned(),
        release_tag: remote.release_tag.clone(),
        asset_name: remote.asset_name.clone(),
        website_version: Some(remote.display_version.clone()),
    })
}

async fn fetch_page_metadata(app: &Application) -> Result<PageMetadata, String> {
    let (page_hosts, _) = stable_hosts(app)?;
    let (official_page_url, marker, link_file_name, segments) = match &app.source {
        SourceSpec::StableDownloadEndpoint {
            official_page_url,
            page_version_marker,
            download_link_file_name,
            page_version_segments,
            ..
        } => (
            official_page_url,
            page_version_marker,
            download_link_file_name,
            *page_version_segments,
        ),
        _ => return Err("该应用不是固定下载地址来源".to_owned()),
    };
    let client = restricted_client(&page_hosts, Duration::from_secs(20))?;
    let response = client
        .get(official_page_url)
        .send()
        .await
        .map_err(|error| format!("读取 {} 官网失败：{error}", app.display_name))?
        .error_for_status()
        .map_err(|error| format!("{} 官网返回错误：{error}", app.display_name))?;
    if response.url().scheme() != "https"
        || !host_allowed(response.url().host_str(), &page_hosts)
    {
        return Err(format!("{} 官网重定向到未授权域名", app.display_name));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_JSON_BYTES)
    {
        return Err(format!("{} 官网页面大小异常", app.display_name));
    }
    let html = response
        .text()
        .await
        .map_err(|error| format!("读取 {} 官网页面内容失败：{error}", app.display_name))?;
    if html.len() as u64 > MAX_JSON_BYTES {
        return Err(format!("{} 官网页面大小异常", app.display_name));
    }
    parse_page_metadata(app, &html, marker, link_file_name, segments)
}

fn parse_page_metadata(
    app: &Application,
    html: &str,
    marker: &str,
    link_file_name: &str,
    segments: usize,
) -> Result<PageMetadata, String> {
    let after_marker = html
        .find(marker)
        .and_then(|index| {
            html[index + marker.len()..]
                .find('>')
                .map(|offset| index + marker.len() + offset + 1)
        })
        .ok_or_else(|| format!("{} 官网页面中未找到版本节点", app.display_name))?;
    let version = html[after_marker..]
        .split('<')
        .next()
        .map(str::trim)
        .filter(|value| valid_display_version(value, segments))
        .ok_or_else(|| format!("{} 官网展示版本格式无效", app.display_name))?
        .to_owned();
    let download_url = extract_href_for_filename(html, link_file_name)
        .ok_or_else(|| format!("{} 官网未提供匹配的 Debian 安装包", app.display_name))?;
    let (_, download_hosts) = stable_hosts(app)?;
    if https_host(&download_url) != download_hosts.first().map(String::as_str) {
        return Err(format!("{} 官网下载链接不属于允许域名", app.display_name));
    }
    Ok(PageMetadata {
        version,
        download_url,
    })
}

fn extract_href_for_filename(html: &str, filename: &str) -> Option<String> {
    let filename_index = html.find(filename)?;
    let before = &html[..filename_index];
    let href_index = before.rfind("href=\"")? + 6;
    Some(html[href_index..].split('"').next()?.to_owned())
}

async fn fetch_release_metadata(app: &Application) -> Result<ReleaseMetadata, String> {
    let (api_hosts, _) = release_hosts(app)?;
    let (release_api_url, pattern, strip_prefix) = match &app.source {
        SourceSpec::ReleaseApi {
            release_api_url,
            asset_name_pattern,
            strip_tag_prefix,
            ..
        } => (release_api_url, asset_name_pattern, strip_tag_prefix),
        _ => return Err("该应用不是发布 API 来源".to_owned()),
    };
    let client = restricted_client(&api_hosts, Duration::from_secs(20))?;
    let response = client
        .get(release_api_url)
        .send()
        .await
        .map_err(|error| format!("读取 {} 发布信息失败：{error}", app.display_name))?
        .error_for_status()
        .map_err(|error| format!("{} 发布 API 返回错误：{error}", app.display_name))?;
    if response.url().scheme() != "https" || !host_allowed(response.url().host_str(), &api_hosts) {
        return Err(format!("{} 发布 API 重定向到未授权域名", app.display_name));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_JSON_BYTES)
    {
        return Err(format!("{} 发布 API 响应大小异常", app.display_name));
    }
    let json = response
        .text()
        .await
        .map_err(|error| format!("读取 {} 发布信息内容失败：{error}", app.display_name))?;
    if json.len() as u64 > MAX_JSON_BYTES {
        return Err(format!("{} 发布 API 响应大小异常", app.display_name));
    }
    parse_release_metadata(app, &json, pattern, strip_prefix.as_deref())
}

fn parse_release_metadata(
    app: &Application,
    json: &str,
    pattern: &str,
    strip_prefix: Option<&str>,
) -> Result<ReleaseMetadata, String> {
    let release: GithubRelease =
        serde_json::from_str(json).map_err(|error| format!("{} 发布信息格式无效：{error}", app.display_name))?;
    if release.draft {
        return Err(format!("{} 最新发布仍为草稿", app.display_name));
    }
    if release.prerelease {
        return Err(format!("{} 最新发布为预发布版本", app.display_name));
    }
    let tag_version = release
        .tag_name
        .strip_prefix(strip_prefix.unwrap_or(""))
        .filter(|value| valid_version_components(value))
        .ok_or_else(|| format!("{} 发布标签格式无效", app.display_name))?
        .to_owned();
    let expected_asset_name = pattern.replace("{tagVersion}", &tag_version);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == expected_asset_name)
        .ok_or_else(|| format!("{} 最新发布未提供匹配的 Debian 资产", app.display_name))?;
    if asset.size == 0 {
        return Err(format!("{} 发布资产大小无效", app.display_name));
    }
    let (_, download_hosts) = release_hosts(app)?;
    let download_host = https_host(&asset.browser_download_url)
        .ok_or_else(|| format!("{} 下载地址格式无效", app.display_name))?;
    if !download_hosts
        .iter()
        .any(|host| host.eq_ignore_ascii_case(download_host))
    {
        return Err(format!("{} 下载地址不属于允许的发布域名", app.display_name));
    }
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| format!("{} 发布资产缺少有效的 SHA-256 摘要", app.display_name))?;

    Ok(ReleaseMetadata {
        tag_name: release.tag_name,
        tag_version,
        asset_name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        expected_size: asset.size,
        sha256,
    })
}

#[derive(Clone, Debug, serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
}

async fn fetch_package_metadata_prefix(
    app: &Application,
    cache_dir: &Path,
    download_url: &str,
) -> Result<PackageMetadata, String> {
    let download_hosts = website_download_hosts(app)?;
    let client = restricted_client(&download_hosts, Duration::from_secs(20))?;
    let probe = fetch_prefix_range(&client, download_url, &download_hosts, CONTROL_PROBE_BYTES).await?;
    let control_end = parse_control_archive_end(&probe.bytes)?;
    if control_end > MAX_CONTROL_PREFIX_BYTES || control_end >= probe.total_size {
        return Err(format!("{} 安装包控制归档大小异常", app.display_name));
    }
    let total_size = probe.total_size;
    let bytes = if control_end <= probe.bytes.len() as u64 {
        probe.bytes[..control_end as usize].to_vec()
    } else {
        let exact = fetch_prefix_range(&client, download_url, &download_hosts, control_end).await?;
        if exact.total_size != total_size {
            return Err(format!("{} 安装包在读取控制信息期间发生变化", app.display_name));
        }
        exact.bytes
    };
    let metadata_dir = cache_dir.join("metadata");
    tokio::fs::create_dir_all(&metadata_dir)
        .await
        .map_err(|error| format!("无法创建元数据缓存目录：{error}"))?;
    let path = metadata_dir.join(format!(
        "{}-control-{}.deb",
        app.package_name,
        std::process::id()
    ));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .await
        .map_err(|error| format!("无法创建控制信息临时文件：{error}"))?;
    file.write_all(&bytes)
        .await
        .map_err(|error| format!("写入控制信息失败：{error}"))?;
    file.flush()
        .await
        .map_err(|error| format!("刷新控制信息失败：{error}"))?;
    drop(file);
    let inspect_path = path.clone();
    let inspection = tauri::async_runtime::spawn_blocking(move || {
        inspect_partial_deb(&inspect_path, total_size, control_end)
    })
    .await
    .map_err(|error| format!("{} 安装包元数据任务失败：{error}", app.display_name))?;
    let cleanup = tokio::fs::remove_file(&path)
        .await
        .map_err(|error| format!("无法清理控制信息临时文件：{error}"));
    let metadata = inspection?;
    cleanup?;
    Ok(metadata)
}

async fn fetch_prefix_range(
    client: &reqwest::Client,
    download_url: &str,
    download_hosts: &[String],
    requested_bytes: u64,
) -> Result<RangePayload, String> {
    if requested_bytes == 0 || requested_bytes > MAX_CONTROL_PREFIX_BYTES {
        return Err("Range 请求大小无效".to_owned());
    }
    let response = client
        .get(download_url)
        .header(RANGE, format!("bytes=0-{}", requested_bytes - 1))
        .send()
        .await
        .map_err(|error| format!("读取安装包控制信息失败：{error}"))?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err("下载服务器未按预期返回 Range 内容，拒绝读取安装包".to_owned());
    }
    if response.url().scheme() != "https"
        || !host_allowed(response.url().host_str(), download_hosts)
    {
        return Err("安装包重定向到未授权域名".to_owned());
    }
    let content_range = parse_content_range(
        response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| "Range 响应缺少 Content-Range".to_owned())?,
    )?;
    if content_range.start != 0 || content_range.end + 1 > requested_bytes {
        return Err("Range 响应范围不符合请求".to_owned());
    }
    if response
        .content_length()
        .is_some_and(|length| length > requested_bytes)
    {
        return Err("Range 响应超过允许大小".to_owned());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取安装包控制信息失败：{error}"))?;
    if bytes.len() as u64 != content_range.end + 1 || bytes.len() as u64 > requested_bytes {
        return Err("Range 响应长度不符合声明".to_owned());
    }
    Ok(RangePayload {
        bytes: bytes.to_vec(),
        total_size: content_range.total,
    })
}

// ---------------------------------------------------------------------------
// Shared download/verify implementation
// ---------------------------------------------------------------------------

async fn apt_download_to_file(
    app: &Application,
    plan: &DownloadPlan,
    path: &Path,
    progress: &ProgressCallback,
) -> Result<VerifiedFile, String> {
    let download_hosts = app.apt_repository_hosts().into_iter().map(str::to_owned).collect::<Vec<_>>();
    let client = restricted_client(&download_hosts, Duration::from_secs(30 * 60))?;
    let mut response = client
        .get(&plan.download_url)
        .send()
        .await
        .map_err(|error| format!("下载 {} 安装包失败：{error}", app.display_name))?
        .error_for_status()
        .map_err(|error| format!("{} 下载服务器返回错误：{error}", app.display_name))?;
    if response.url().scheme() != "https"
        || !host_allowed(response.url().host_str(), &download_hosts)
    {
        return Err(format!("{} 安装包最终下载地址不属于允许域名", app.display_name));
    }
    if response
        .content_length()
        .is_some_and(|length| length != plan.expected_size)
    {
        return Err(format!("{} 安装包响应大小与官方索引不一致", app.display_name));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|error| format!("无法创建临时下载文件：{error}"))?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut last_at = Instant::now();
    let mut last_bytes = 0_u64;
    emit_progress(progress, plan, "downloading", 0, 0);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取安装包下载内容失败：{error}"))?
    {
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| "安装包大小溢出".to_owned())?;
        if size > plan.expected_size {
            return Err("安装包超过官方索引声明的总大小".to_owned());
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("写入安装包失败：{error}"))?;
        let elapsed = last_at.elapsed();
        if elapsed >= Duration::from_millis(250) || size == plan.expected_size {
            let speed = if elapsed.is_zero() {
                0
            } else {
                (size - last_bytes) * 1000 / elapsed.as_millis().max(1) as u64
            };
            emit_progress(progress, plan, "downloading", size, speed);
            last_at = Instant::now();
            last_bytes = size;
        }
    }
    file.flush()
        .await
        .map_err(|error| format!("刷新安装包失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("同步安装包失败：{error}"))?;
    drop(file);

    if size != plan.expected_size {
        return Err("安装包实际大小与官方索引声明不一致".to_owned());
    }
    let sha256 = format!("{:x}", hasher.finalize());
    if let Some(expected) = plan.expected_sha256.as_deref()
        && !sha256.eq_ignore_ascii_case(expected)
    {
        return Err("安装包 SHA-256 与官方索引记录不一致".to_owned());
    }
    emit_progress(progress, plan, "verifying", size, 0);
    let metadata = inspect_deb(path).await?;
    validate_apt_downloaded_metadata(plan, &metadata)?;
    Ok((size, sha256, metadata))
}

async fn apt_verify_file(
    _app: &Application,
    plan: &DownloadPlan,
    path: &Path,
    progress: Option<&ProgressCallback>,
) -> Result<VerifiedFile, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("无法读取缓存安装包：{error}"))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut last_at = Instant::now();
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取缓存安装包失败：{error}"))?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > plan.expected_size {
            return Err("缓存安装包大小异常".to_owned());
        }
        hasher.update(&buffer[..count]);
        if let Some(progress) = progress
            && (last_at.elapsed() >= Duration::from_millis(100) || size == plan.expected_size)
        {
            emit_progress(progress, plan, "verifying", size, 0);
            last_at = Instant::now();
        }
    }
    if size != plan.expected_size {
        return Err("缓存安装包大小与官方索引声明不一致".to_owned());
    }
    let sha256 = format!("{:x}", hasher.finalize());
    if let Some(expected) = plan.expected_sha256.as_deref()
        && !sha256.eq_ignore_ascii_case(expected)
    {
        return Err("缓存安装包 SHA-256 与官方索引记录不一致".to_owned());
    }
    let metadata = inspect_deb(path).await?;
    validate_apt_downloaded_metadata(plan, &metadata)?;
    Ok((size, sha256, metadata))
}

fn validate_apt_downloaded_metadata(plan: &DownloadPlan, metadata: &DebMetadata) -> Result<(), String> {
    if metadata.package != plan.package_name
        || metadata.version != plan.version
        || metadata.architecture != plan.architecture
    {
        return Err("安装包 .deb 元数据与下载计划不一致".to_owned());
    }
    Ok(())
}

async fn download_website_to_file(
    app: &Application,
    plan: &DownloadPlan,
    path: &Path,
    progress: &ProgressCallback,
) -> Result<VerifiedFile, String> {
    let download_hosts = website_download_hosts(app)?;
    let client = restricted_client(&download_hosts, Duration::from_secs(30 * 60))?;
    let mut response = client
        .get(&plan.download_url)
        .send()
        .await
        .map_err(|error| format!("下载 {} 安装包失败：{error}", app.display_name))?
        .error_for_status()
        .map_err(|error| format!("{} 下载服务器返回错误：{error}", app.display_name))?;
    if response.url().scheme() != "https"
        || !host_allowed(response.url().host_str(), &download_hosts)
    {
        return Err(format!("{} 安装包最终下载地址不属于允许域名", app.display_name));
    }
    if response
        .content_length()
        .is_some_and(|length| length != plan.expected_size)
    {
        return Err(format!("{} 安装包响应大小与声明不一致", app.display_name));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|error| format!("无法创建临时下载文件：{error}"))?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut last_at = Instant::now();
    let mut last_bytes = 0_u64;
    emit_progress(progress, plan, "downloading", 0, 0);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取安装包下载内容失败：{error}"))?
    {
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| "安装包大小溢出".to_owned())?;
        if size > plan.expected_size {
            return Err("安装包超过声明的总大小".to_owned());
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("写入安装包失败：{error}"))?;
        let elapsed = last_at.elapsed();
        if elapsed >= Duration::from_millis(250) || size == plan.expected_size {
            let speed = if elapsed.is_zero() {
                0
            } else {
                (size - last_bytes) * 1000 / elapsed.as_millis().max(1) as u64
            };
            emit_progress(progress, plan, "downloading", size, speed);
            last_at = Instant::now();
            last_bytes = size;
        }
    }
    file.flush()
        .await
        .map_err(|error| format!("刷新安装包失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("同步安装包失败：{error}"))?;
    drop(file);

    if size != plan.expected_size {
        return Err("安装包实际大小与声明不一致".to_owned());
    }
    let sha256 = format!("{:x}", hasher.finalize());
    if let Some(expected) = plan.expected_sha256.as_deref()
        && !sha256.eq_ignore_ascii_case(expected)
    {
        return Err("安装包 SHA-256 与发布摘要不一致".to_owned());
    }
    emit_progress(progress, plan, "verifying", size, 0);
    let metadata = inspect_deb(path).await?;
    validate_website_downloaded_metadata(app, plan, &metadata)?;
    Ok((size, sha256, metadata))
}

async fn verify_website_file(
    app: &Application,
    plan: &DownloadPlan,
    path: &Path,
    progress: Option<&ProgressCallback>,
) -> Result<VerifiedFile, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("无法读取缓存安装包：{error}"))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut last_at = Instant::now();
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取缓存安装包失败：{error}"))?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > plan.expected_size {
            return Err("缓存安装包大小异常".to_owned());
        }
        hasher.update(&buffer[..count]);
        if let Some(progress) = progress
            && (last_at.elapsed() >= Duration::from_millis(100) || size == plan.expected_size)
        {
            emit_progress(progress, plan, "verifying", size, 0);
            last_at = Instant::now();
        }
    }
    if size != plan.expected_size {
        return Err("缓存安装包大小与声明不一致".to_owned());
    }
    let sha256 = format!("{:x}", hasher.finalize());
    if let Some(expected) = plan.expected_sha256.as_deref()
        && !sha256.eq_ignore_ascii_case(expected)
    {
        return Err("缓存安装包 SHA-256 与发布摘要不一致".to_owned());
    }
    let metadata = inspect_deb(path).await?;
    validate_website_downloaded_metadata(app, plan, &metadata)?;
    Ok((size, sha256, metadata))
}

fn validate_website_downloaded_metadata(
    app: &Application,
    plan: &DownloadPlan,
    metadata: &DebMetadata,
) -> Result<(), String> {
    let display_version = plan.website_version.as_deref().unwrap_or("");
    if metadata.package != plan.package_name
        || metadata.version != plan.version
        || metadata.architecture != plan.architecture
        || !website_version_matches(app, &metadata.version, display_version)
    {
        return Err(format!("{} 安装包元数据与下载计划不一致", app.display_name));
    }
    Ok(())
}

fn website_version_matches(app: &Application, package_version: &str, display_version: &str) -> bool {
    match remote_kind(app) {
        RemoteKind::StableDownload => version_has_display_prefix(package_version, display_version),
        RemoteKind::ReleaseApi => version_has_release_prefix(package_version, display_version),
    }
}

async fn inspect_deb(path: &Path) -> Result<DebMetadata, String> {
    let owned_path = path.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(DebMetadata {
            package: read_deb_field(&owned_path, "Package")?,
            version: read_deb_field(&owned_path, "Version")?,
            architecture: read_deb_field(&owned_path, "Architecture")?,
        })
    })
    .await
    .map_err(|error| format!("安装包元数据检查任务失败：{error}"))?
}

fn inspect_partial_deb(
    path: &Path,
    total_size: u64,
    metadata_bytes: u64,
) -> Result<PackageMetadata, String> {
    let package_name = read_deb_field(path, "Package")?;
    let version = read_deb_field(path, "Version")?;
    let architecture = read_deb_field(path, "Architecture")?;
    Ok(PackageMetadata {
        package_name,
        version,
        architecture,
        total_size,
        metadata_bytes,
    })
}

fn read_deb_field(path: &Path, field: &str) -> Result<String, String> {
    let output = clean_command(DPKG_DEB_BIN)
        .arg("--field")
        .arg(path)
        .arg(field)
        .output()
        .map_err(|error| format!("无法读取 .deb 字段 {field}：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            ".deb 字段 {field} 无效：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if value.is_empty() {
        return Err(format!(".deb 缺少 {field} 字段"));
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Local dpkg helpers
// ---------------------------------------------------------------------------

fn installed_package_version_optional(app: &Application) -> Result<Option<String>, String> {
    let output = clean_command(DPKG_QUERY_BIN)
        .args([
            "-W",
            "-f=${db:Status-Abbrev}\t${Version}\t${Architecture}",
            &app.package_name,
        ])
        .output()
        .map_err(|error| format!("无法查询 {} 安装状态：{error}", app.display_name))?;
    if !output.status.success() {
        return Ok(None);
    }
    parse_installed_version_optional(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        app.architecture.as_str(),
    )
}

fn parse_installed_version_optional(
    rendered: &str,
    expected_architecture: &str,
) -> Result<Option<String>, String> {
    let mut fields = rendered.split('\t');
    let status = fields.next();
    let version = fields.next();
    let architecture = fields.next();
    if fields.next().is_some() {
        return Err("dpkg 安装状态格式无效".to_owned());
    }
    if status == Some("ii ") && architecture == Some(expected_architecture) {
        let value = version.filter(|value| !value.is_empty());
        if value.is_some() {
            return Ok(value.map(str::to_owned));
        }
    }
    Ok(None)
}

fn debian_version_is_newer(installed: &str, candidate: &str) -> bool {
    clean_command(DPKG_BIN)
        .args(["--compare-versions", installed, "lt", candidate])
        .status()
        .is_ok_and(|status| status.success())
}

// ---------------------------------------------------------------------------
// Version and format helpers
// ---------------------------------------------------------------------------

fn parse_control_archive_end(bytes: &[u8]) -> Result<u64, String> {
    if !bytes.starts_with(b"!<arch>\n") {
        return Err("安装包缺少 Debian ar 文件头".to_owned());
    }
    let mut offset = 8_usize;
    while offset + 60 <= bytes.len() {
        let header = &bytes[offset..offset + 60];
        if &header[58..60] != b"`\n" {
            return Err("安装包 ar 成员头无效".to_owned());
        }
        let name = std::str::from_utf8(&header[..16])
            .map_err(|_| "安装包 ar 成员名称无效".to_owned())?
            .trim()
            .trim_end_matches('/');
        let size = std::str::from_utf8(&header[48..58])
            .ok()
            .map(str::trim)
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| "安装包 ar 成员大小无效".to_owned())?;
        let data_start = offset
            .checked_add(60)
            .ok_or_else(|| "安装包 ar 偏移溢出".to_owned())?;
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| "安装包 ar 成员大小溢出".to_owned())?;
        let padded_end = data_end
            .checked_add(size % 2)
            .ok_or_else(|| "安装包 ar 对齐大小溢出".to_owned())?;
        if name.starts_with("control.tar") {
            return u64::try_from(padded_end).map_err(|_| "安装包控制归档大小溢出".to_owned());
        }
        if padded_end > bytes.len() {
            return Err("探测范围内未找到安装包控制归档".to_owned());
        }
        offset = padded_end;
    }
    Err("安装包中未找到控制归档".to_owned())
}

fn parse_content_range(value: &str) -> Result<ContentRange, String> {
    let value = value
        .strip_prefix("bytes ")
        .ok_or_else(|| "Content-Range 格式无效".to_owned())?;
    let (range, total) = value
        .split_once('/')
        .ok_or_else(|| "Content-Range 格式无效".to_owned())?;
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| "Content-Range 格式无效".to_owned())?;
    let parsed = ContentRange {
        start: start
            .parse()
            .map_err(|_| "Content-Range 起点无效".to_owned())?,
        end: end
            .parse()
            .map_err(|_| "Content-Range 终点无效".to_owned())?,
        total: total
            .parse()
            .map_err(|_| "Content-Range 总大小无效".to_owned())?,
    };
    if parsed.end < parsed.start || parsed.total <= parsed.end {
        return Err("Content-Range 数值无效".to_owned());
    }
    Ok(parsed)
}

fn valid_version_components(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.split('.').any(str::is_empty)
}

fn valid_display_version(value: &str, segments: usize) -> bool {
    let components: Vec<_> = value.split('.').collect();
    components.len() == segments
        && components.iter().all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn version_has_display_prefix(package_version: &str, display_version: &str) -> bool {
    package_version == display_version
        || package_version
            .strip_prefix(display_version)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn version_has_release_prefix(package_version: &str, release_version: &str) -> bool {
    package_version == release_version
        || package_version
            .strip_prefix(release_version)
            .is_some_and(|suffix| {
                suffix.starts_with('.')
                    || suffix.starts_with('+')
                    || suffix.starts_with('-')
                    || suffix.starts_with('~')
            })
}

fn https_host(url: &str) -> Option<&str> {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
}

fn host_allowed(host: Option<&str>, allowed_hosts: &[String]) -> bool {
    host.is_some_and(|host| {
        allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    })
}

// ---------------------------------------------------------------------------
// HTTP client and process helpers
// ---------------------------------------------------------------------------

fn restricted_client(allowed_hosts: &[String], timeout: Duration) -> Result<reqwest::Client, String> {
    let hosts = allowed_hosts.to_vec();
    crate::network::apply_proxy(reqwest::Client::builder())
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("请求重定向次数超过限制");
            }
            if attempt.url().scheme() == "https"
                && hosts
                    .iter()
                    .any(|host| attempt.url().host_str() == Some(host.as_str()))
            {
                attempt.follow()
            } else {
                attempt.error("请求重定向到未授权域名")
            }
        }))
        .user_agent("UManager/0.1")
        .build()
        .map_err(|error| format!("无法创建安全请求客户端：{error}"))
}

fn emit_progress(
    callback: &ProgressCallback,
    plan: &DownloadPlan,
    phase: &'static str,
    transferred_bytes: u64,
    bytes_per_second: u64,
) {
    callback(DownloadProgress {
        package_name: plan.package_name.clone(),
        phase,
        transferred_bytes,
        total_bytes: plan.expected_size,
        bytes_per_second,
    });
}

fn clean_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", SAFE_SYSTEM_PATH)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    command
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

    fn flclash_app() -> Application {
        umanager_catalog::Catalog::load()
            .unwrap()
            .by_application_id("flclash")
            .unwrap()
            .clone()
    }

    fn wechat_app() -> Application {
        umanager_catalog::Catalog::load()
            .unwrap()
            .by_application_id("wechat")
            .unwrap()
            .clone()
    }

    #[test]
    fn matches_only_a_complete_version_prefix() {
        assert!(version_has_display_prefix("4.1.1.8", "4.1.1"));
        assert!(version_has_display_prefix("4.1.1", "4.1.1"));
        assert!(!version_has_display_prefix("4.1.10.1", "4.1.1"));
        assert!(version_has_release_prefix("0.8.96+2026081701", "0.8.96"));
        assert!(!version_has_release_prefix("0.8.960", "0.8.96"));
    }

    #[test]
    fn parses_the_server_rendered_page_for_configured_markers() {
        let app = wechat_app();
        let (marker, link_file, segments) = match &app.source {
            SourceSpec::StableDownloadEndpoint {
                page_version_marker,
                download_link_file_name,
                page_version_segments,
                ..
            } => (page_version_marker, download_link_file_name, *page_version_segments),
            _ => unreachable!(),
        };
        let html = r#"<div class="main-section__bd-version" data-v-x>4.1.1</div><a href="https://dldir1v6.qq.com/weixin/Universal/Linux/WeChatLinux_x86_64.deb">deb</a>"#;
        let page = parse_page_metadata(&app, html, marker, link_file, segments).unwrap();
        assert_eq!(page.version, "4.1.1");
        assert!(page.download_url.ends_with("WeChatLinux_x86_64.deb"));
    }

    #[test]
    fn selects_the_exact_configured_release_asset() {
        let app = flclash_app();
        let json = r#"{
  "tag_name": "v0.8.96",
  "draft": false,
  "prerelease": false,
  "assets": [
    { "name": "FlClash-0.8.96-android.apk", "size": 1, "digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "browser_download_url": "https://github.com/chen08209/FlClash/releases/download/v0.8.96/a.apk" },
    { "name": "FlClash-0.8.96-linux-amd64.deb", "size": 42, "digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", "browser_download_url": "https://github.com/chen08209/FlClash/releases/download/v0.8.96/FlClash-0.8.96-linux-amd64.deb" }
  ]
}"#;
        let metadata =
            parse_release_metadata(&app, json, "FlClash-{tagVersion}-linux-amd64.deb", Some("v"))
                .unwrap();
        assert_eq!(metadata.tag_name, "v0.8.96");
        assert_eq!(metadata.asset_name, "FlClash-0.8.96-linux-amd64.deb");
        assert_eq!(metadata.sha256.len(), 64);
    }

    #[test]
    fn parses_content_range() {
        assert_eq!(
            parse_content_range("bytes 0-4095/42085932").unwrap(),
            ContentRange {
                start: 0,
                end: 4095,
                total: 42_085_932,
            }
        );
        assert!(parse_content_range("bytes 0-100/*").is_err());
    }

    #[test]
    fn locates_the_exact_control_archive_end_from_the_ar_directory() {
        fn append_member(archive: &mut Vec<u8>, name: &str, data: &[u8]) {
            archive.extend(
                format!(
                    "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                    0,
                    0,
                    0,
                    100644,
                    data.len()
                )
                .as_bytes(),
            );
            archive.extend(data);
            if data.len() % 2 == 1 {
                archive.push(b'\n');
            }
        }
        let mut archive = b"!<arch>\n".to_vec();
        append_member(&mut archive, "debian-binary/", b"2.0\n");
        append_member(&mut archive, "control.tar.zst/", b"control");
        let expected = archive.len() as u64;
        append_member(&mut archive, "data.tar.zst/", b"payload");
        assert_eq!(parse_control_archive_end(&archive).unwrap(), expected);
    }

    #[test]
    fn parses_installed_version_and_only_fully_installed_amd64() {
        assert_eq!(
            parse_installed_version_optional("ii \t0.8.96+2026081701\tamd64", "amd64").unwrap(),
            Some("0.8.96+2026081701".to_owned())
        );
        assert!(parse_installed_version_optional("rc \t0.8.96+2026081701\tamd64", "amd64")
            .unwrap()
            .is_none());
        assert!(parse_installed_version_optional("ii \t0.8.96+2026081701\tarm64", "amd64")
            .unwrap()
            .is_none());
    }

    #[test]
    #[ignore = "requires network and a working system proxy to reach the GitHub asset"]
    fn fetches_flclash_control_prefix_through_the_system_proxy() {
        let app = flclash_app();
        let download_hosts = website_download_hosts(&app).unwrap();
        let url = "https://github.com/chen08209/FlClash/releases/download/v0.8.96/FlClash-0.8.96-linux-amd64.deb";
        let client = restricted_client(&download_hosts, Duration::from_secs(30)).unwrap();
        let payload = tauri::async_runtime::block_on(fetch_prefix_range(
            &client,
            url,
            &download_hosts,
            CONTROL_PROBE_BYTES,
        ))
        .unwrap();
        assert!(payload.total_size > 0);
        assert!(parse_control_archive_end(&payload.bytes).is_ok());
    }
}
