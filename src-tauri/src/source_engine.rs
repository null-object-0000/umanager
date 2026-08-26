use crate::feed::FeedApplicationEntry;
use crate::scanner::{SourceKind, UpdateState};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use umanager_catalog::{Application, SourceSpec};

const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";
const DPKG_BIN: &str = "/usr/bin/dpkg";
const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

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
    pub version_updated_at_unix_seconds: Option<u64>,
    pub version_updated_at_source: Option<crate::feed::VersionUpdatedAtSource>,
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
    pub(crate) version_updated_at_unix_seconds: Option<u64>,
    pub(crate) version_updated_at_source: Option<crate::feed::VersionUpdatedAtSource>,
}

#[derive(Clone, Debug)]
struct DebMetadata {
    package: String,
    version: String,
    architecture: String,
}

type VerifiedFile = (u64, String, DebMetadata);

// ---------------------------------------------------------------------------
// Public entry points — all software metadata now comes exclusively from the
// central feed. The download + verification engine below is unchanged in spirit:
// it downloads from the pinned official URL and re-validates size, SHA-256 and
// the .deb package name/version/architecture before an immutable plan is created.
// ---------------------------------------------------------------------------

pub async fn load_details(app: &Application, cache_dir: &Path) -> Result<ApplicationDetails, String> {
    let entry = required_feed_entry(app).await?;
    feed_details(app, cache_dir, &entry)
}

pub async fn build_download_plan(app: &Application, cache_dir: &Path) -> Result<DownloadPlan, String> {
    let entry = required_feed_entry(app).await?;
    feed_plan(app, cache_dir, &entry)
}

pub(crate) async fn load_installable(app: &Application, cache_dir: &Path) -> Result<Installable, String> {
    let installed = installed_package_version_optional(app)?;
    let Some(entry) = optional_feed_entry(app).await? else {
        // No version data in the feed yet (e.g. CI couldn't scrape a vendor site);
        // mark it as not installable so it never breaks the whole store list.
        return Ok(Installable {
            installed_version: installed,
            candidate_version: None,
            download_plan: None,
            version_updated_at_unix_seconds: None,
            version_updated_at_source: None,
        });
    };
    feed_installable(app, cache_dir, &entry, installed)
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

// ---------------------------------------------------------------------------
// Feed-backed details / plan construction
// ---------------------------------------------------------------------------

async fn optional_feed_entry(app: &Application) -> Result<Option<FeedApplicationEntry>, String> {
    if app.application_id == "umanager" {
        crate::feed::self_update_entry().await
    } else {
        crate::feed::entry_for(app).await
    }
}

async fn required_feed_entry(app: &Application) -> Result<FeedApplicationEntry, String> {
    optional_feed_entry(app)
        .await?
        .ok_or_else(|| format!("元数据源中缺少 {} 的最新版本信息", app.display_name))
}

fn feed_details(
    app: &Application,
    _cache_dir: &Path,
    entry: &FeedApplicationEntry,
) -> Result<ApplicationDetails, String> {
    validate_feed_entry_for_app(app, entry)?;
    let installed = installed_package_version_optional(app)?;
    let update_state = match installed.as_deref() {
        Some(installed) if debian_version_is_newer(installed, &entry.version) => {
            UpdateState::UpdateAvailable
        }
        Some(_) => UpdateState::UpToDate,
        None => UpdateState::Unknown,
    };
    let is_apt = matches!(app.source, SourceSpec::AptRepository { .. });
    let kind = if is_apt {
        SourceKind::OfficialRepository
    } else {
        SourceKind::OfficialWebsite
    };
    let display_version = entry
        .website_version
        .clone()
        .unwrap_or_else(|| entry.version.clone());
    Ok(ApplicationDetails {
        application_id: app.application_id.clone(),
        display_name: app.display_name.clone(),
        package_name: entry.package_name.clone(),
        vendor: app.vendor.clone(),
        architecture: entry.architecture.clone(),
        source_kind: kind,
        source_url: entry.download_url.clone(),
        installed_version: installed,
        candidate_version: Some(entry.version.clone()),
        update_state,
        website_version: if is_apt { None } else { Some(display_version) },
        expected_size: Some(entry.size),
        sha256: Some(entry.sha256.clone()),
        metadata_bytes: None,
        release_tag: entry.release_tag.clone(),
        asset_name: entry.asset_name.clone(),
        version_updated_at_unix_seconds: entry.version_updated_at_unix_seconds,
        version_updated_at_source: entry.version_updated_at_source,
        trusted: true,
        evidence: vec![
            Evidence {
                label: "元数据来源".to_owned(),
                actual: "UManager 官方采集镜像（Ed25519 签名）".to_owned(),
                expected: "UManager 官方采集镜像（Ed25519 签名）".to_owned(),
                passed: true,
            },
            Evidence {
                label: "下载域名".to_owned(),
                actual: app.download_hosts().join(", "),
                expected: app.download_hosts().join(", "),
                passed: true,
            },
            Evidence {
                label: "Debian 软件包名".to_owned(),
                actual: entry.package_name.clone(),
                expected: app.package_name.clone(),
                passed: entry.package_name == app.package_name,
            },
            Evidence {
                label: "软件包架构".to_owned(),
                actual: entry.architecture.clone(),
                expected: app.architecture.clone(),
                passed: entry.architecture == app.architecture,
            },
        ],
    })
}

fn feed_plan(
    app: &Application,
    cache_dir: &Path,
    entry: &FeedApplicationEntry,
) -> Result<DownloadPlan, String> {
    validate_feed_entry_for_app(app, entry)?;
    let is_apt = matches!(app.source, SourceSpec::AptRepository { .. });
    let kind = if is_apt {
        SourceKind::OfficialRepository
    } else {
        SourceKind::OfficialWebsite
    };
    let version_hash = format!("{:x}", Sha256::digest(entry.version.as_bytes()));
    let file_name = format!("{}-{}.deb", entry.package_name, &version_hash[..16]);
    let target_path = cache_dir
        .join("downloads")
        .join(&file_name)
        .to_string_lossy()
        .into_owned();
    Ok(DownloadPlan {
        application_id: app.application_id.clone(),
        package_name: entry.package_name.clone(),
        version: entry.version.clone(),
        architecture: entry.architecture.clone(),
        source_kind: kind,
        repository_url: if is_apt {
            app.apt_repository_url().map(str::to_owned)
        } else {
            None
        },
        download_url: entry.download_url.clone(),
        file_name: file_name.clone(),
        expected_size: entry.size,
        expected_sha256: Some(entry.sha256.clone()),
        target_path,
        release_tag: entry.release_tag.clone(),
        asset_name: entry.asset_name.clone(),
        website_version: entry.website_version.clone(),
    })
}

fn feed_installable(
    app: &Application,
    cache_dir: &Path,
    entry: &FeedApplicationEntry,
    installed: Option<String>,
) -> Result<Installable, String> {
    if installed.is_some() {
        return Ok(Installable {
            installed_version: installed,
            candidate_version: Some(entry.version.clone()),
            download_plan: None,
            version_updated_at_unix_seconds: entry.version_updated_at_unix_seconds,
            version_updated_at_source: entry.version_updated_at_source,
        });
    }
    Ok(Installable {
        installed_version: None,
        candidate_version: Some(entry.version.clone()),
        download_plan: Some(feed_plan(app, cache_dir, entry)?),
        version_updated_at_unix_seconds: entry.version_updated_at_unix_seconds,
        version_updated_at_source: entry.version_updated_at_source,
    })
}

fn validate_feed_entry_for_app(app: &Application, entry: &FeedApplicationEntry) -> Result<(), String> {
    if entry.package_name != app.package_name || entry.architecture != app.architecture {
        return Err(format!(
            "元数据源中 {} 的包名或架构与软件源不一致",
            app.display_name
        ));
    }
    let hosts = app.download_hosts();
    let host = https_host(&entry.download_url).ok_or_else(|| "元数据源下载地址格式无效".to_owned())?;
    if !hosts.iter().any(|allowed| host_matches(host, allowed)) {
        return Err(format!(
            "元数据源下载地址不属于 {} 的允许域名",
            app.display_name
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared download + verify implementation
// ---------------------------------------------------------------------------

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

/// Resolve the download URL to actually fetch: re-fetch the version endpoint if
/// `resolve_at_download` is set (Feishu's links expire), then apply any URL
/// signing step (QQ). Other sources return the URL unchanged.
async fn resolve_download_url(app: &Application, raw_url: &str) -> Result<String, String> {
    let SourceSpec::VersionEndpoint {
        version_endpoint_url,
        version_endpoint_hosts,
        query,
        payload_kind,
        download_url_field,
        download_hosts,
        sign,
        resolve_at_download,
        ..
    } = &app.source
    else {
        return Ok(raw_url.to_owned());
    };

    let mut url = raw_url.to_owned();
    if *resolve_at_download {
        url = re_resolve_endpoint_url(
            version_endpoint_url,
            version_endpoint_hosts,
            query,
            payload_kind,
            download_url_field,
        )
        .await?;
    }
    if let Some(sign_config) = sign {
        url = sign_url(sign_config, &url).await?;
    }
    let hosts = download_hosts;
    if !url.starts_with("https://") || !host_allowed(https_host(&url), hosts) {
        return Err("下载地址不属于允许域名".to_owned());
    }
    Ok(url)
}

/// Apply the configured URL-signing step (e.g. QQ's trpc UrlSign) to `raw_url`.
async fn sign_url(sign: &umanager_catalog::VersionEndpointSign, raw_url: &str) -> Result<String, String> {
    let client = restricted_client(&sign.endpoint_hosts, Duration::from_secs(20))?;
    let body = sign.body_template.replace("{downloadUrl}", raw_url);
    let mut request = client.post(&sign.endpoint_url);
    request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
    if let Some(headers) = &sign.headers
        && let Some(map) = headers.as_object()
    {
        for (key, value) in map {
            if let Some(text) = value.as_str() {
                request = request.header(key.as_str(), text);
            }
        }
    }
    let response = request
        .body(body)
        .send()
        .await
        .map_err(|error| format!("获取签名下载地址失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("签名接口返回错误：{error}"))?;
    if response.url().scheme() != "https"
        || !host_allowed(response.url().host_str(), &sign.endpoint_hosts)
    {
        return Err("签名接口重定向到未授权域名".to_owned());
    }
    let body_text = response
        .text()
        .await
        .map_err(|error| format!("读取签名接口响应失败：{error}"))?;
    let json: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|error| format!("签名接口响应无效：{error}"))?;
    let pointer = format!("/{}", sign.signed_url_field.replace('.', "/"));
    json.pointer(&pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "签名接口未返回下载地址".to_owned())
}

// Dot-path access with array-index support, mirroring the generator's getJsonPath.
fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        if let Ok(index) = segment.parse::<usize>() {
            current = current.get(index)?;
        } else {
            current = current.get(segment)?;
        }
    }
    Some(current)
}

// Extract a balanced JSON object from inside a JS file (e.g. `var params = {...};`).
fn extract_json_object(text: &str) -> Result<&str, String> {
    let start = text.find('{').ok_or_else(|| "未找到 JSON 对象".to_owned())?;
    let bytes = text.as_bytes();
    let mut depth = 0_i32;
    let mut in_str = false;
    let mut esc = false;
    for (index, &byte) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if byte == b'\\' {
                esc = true;
            } else if byte == b'"' {
                in_str = false;
            }
        } else if byte == b'"' {
            in_str = true;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return Ok(&text[start..=index]);
            }
        }
    }
    Err("JSON 对象不完整".to_owned())
}

// Re-fetch the version endpoint and extract a fresh download URL (Feishu etc.).
async fn re_resolve_endpoint_url(
    version_endpoint_url: &str,
    hosts: &[String],
    query: &Option<serde_json::Map<String, serde_json::Value>>,
    payload_kind: &umanager_catalog::VersionEndpointPayload,
    download_url_field: &str,
) -> Result<String, String> {
    let endpoint = build_endpoint_url(version_endpoint_url, query)?;
    let client = restricted_client(hosts, Duration::from_secs(20))?;
    let response = client
        .get(&endpoint)
        .send()
        .await
        .map_err(|error| format!("重新解析下载地址失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("版本端点返回错误：{error}"))?;
    if response.url().scheme() != "https" || !host_allowed(response.url().host_str(), hosts) {
        return Err("版本端点重定向到未授权域名".to_owned());
    }
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取版本端点响应失败：{error}"))?;
    let payload = match payload_kind {
        umanager_catalog::VersionEndpointPayload::Json => serde_json::from_str(&text),
        umanager_catalog::VersionEndpointPayload::JsonInScript => {
            serde_json::from_str(extract_json_object(&text)?)
        }
        umanager_catalog::VersionEndpointPayload::Html => {
            return Err("HTML 端点不支持下载时重新解析".to_owned());
        }
    }
    .map_err(|error| format!("版本端点响应无效：{error}"))?;
    json_path(&payload, download_url_field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "版本端点未返回下载地址".to_owned())
}

fn build_endpoint_url(
    base: &str,
    query: &Option<serde_json::Map<String, serde_json::Value>>,
) -> Result<String, String> {
    let mut url = url::Url::parse(base).map_err(|error| format!("版本端点地址无效：{error}"))?;
    if let Some(params) = query {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in params {
            let rendered = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            pairs.append_pair(key, &rendered);
        }
    }
    Ok(url.to_string())
}

async fn download_plan_file(
    app: &Application,
    plan: &DownloadPlan,
    path: &Path,
    progress: &ProgressCallback,
) -> Result<VerifiedFile, String> {
    let download_hosts = app.download_hosts().into_iter().map(str::to_owned).collect::<Vec<_>>();
    let download_url = resolve_download_url(app, &plan.download_url).await?;
    let client = restricted_client(&download_hosts, Duration::from_secs(30 * 60))?;
    let mut response = client
        .get(&download_url)
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
        return Err(format!("{} 安装包响应大小与元数据源声明不一致", app.display_name));
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
            return Err("安装包超过元数据源声明的总大小".to_owned());
        }
        hasher.update(&chunk);
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
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
        return Err("安装包实际大小与元数据源声明不一致".to_owned());
    }
    let sha256 = format!("{:x}", hasher.finalize());
    let expected = plan
        .expected_sha256
        .as_deref()
        .ok_or_else(|| "元数据源缺少该安装包的 SHA-256".to_owned())?;
    if !sha256.eq_ignore_ascii_case(expected) {
        return Err("安装包 SHA-256 与元数据源记录不一致".to_owned());
    }
    emit_progress(progress, plan, "verifying", size, 0);
    let metadata = inspect_deb(path).await?;
    validate_downloaded_metadata(plan, &metadata)?;
    Ok((size, sha256, metadata))
}

async fn verify_plan_file(
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
        return Err("缓存安装包大小与元数据源声明不一致".to_owned());
    }
    let sha256 = format!("{:x}", hasher.finalize());
    let expected = plan
        .expected_sha256
        .as_deref()
        .ok_or_else(|| "元数据源缺少该安装包的 SHA-256".to_owned())?;
    if !sha256.eq_ignore_ascii_case(expected) {
        return Err("缓存安装包 SHA-256 与元数据源记录不一致".to_owned());
    }
    let metadata = inspect_deb(path).await?;
    validate_downloaded_metadata(plan, &metadata)?;
    Ok((size, sha256, metadata))
}

fn validate_downloaded_metadata(plan: &DownloadPlan, metadata: &DebMetadata) -> Result<(), String> {
    if metadata.package != plan.package_name
        || metadata.version != plan.version
        || metadata.architecture != plan.architecture
    {
        return Err("安装包 .deb 元数据与下载计划不一致".to_owned());
    }
    Ok(())
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
// HTTP client and process helpers
// ---------------------------------------------------------------------------

/// Whether `host` is accepted by `allowed`. A `*.<domain>` allowed entry also
/// matches the exact domain and any of its subdomains; the leading `.` prevents
/// suffix collisions (e.g. `*.feishucdn.com` does not accept `evilfeishucdn.com`).
/// This is the narrow, documented exception for vendors whose download CDN shards
/// a stable root domain (e.g. Feishu's `lf?-ug-sign.feishucdn.com`).
pub(crate) fn host_matches(host: &str, allowed: &str) -> bool {
    if let Some(domain) = allowed.strip_prefix("*.") {
        let host = host.to_ascii_lowercase();
        let domain = domain.to_ascii_lowercase();
        host == domain || host.ends_with(&format!(".{domain}"))
    } else {
        host.eq_ignore_ascii_case(allowed)
    }
}

pub(crate) fn host_allowed(host: Option<&str>, allowed_hosts: &[String]) -> bool {
    host.is_some_and(|host| {
        allowed_hosts
            .iter()
            .any(|allowed| host_matches(host, allowed))
    })
}

pub(crate) fn https_host(url: &str) -> Option<&str> {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
}

pub(crate) fn restricted_client(allowed_hosts: &[String], timeout: Duration) -> Result<reqwest::Client, String> {
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
                    .any(|host| attempt.url().host_str().is_some_and(|h| host_matches(h, host)))
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

    #[test]
    fn host_allowed_exact_plus_documented_subdomain_exception() {
        let exact = |h: &str| host_allowed(Some(h), &["qqdl.gtimg.cn".to_owned()]);
        // Exact whitelist still matches only the exact host.
        assert!(exact("qqdl.gtimg.cn"));
        assert!(!exact("sub.qqdl.gtimg.cn"));
        assert!(!exact("qqdl.gtimg.cn.evil.com"));

        let feishu = |h: &str| host_allowed(Some(h), &["*.feishucdn.com".to_owned()]);
        // `*.<domain>` accepts the root and any subdomain, but not lookalikes.
        assert!(feishu("feishucdn.com"));
        assert!(feishu("lf6-ug-sign.feishucdn.com"));
        assert!(feishu("lf3-ug-sign.feishucdn.com"));
        assert!(!feishu("feishucdn.com.evil.com"));
        assert!(!feishu("notfeishucdn.com"));
        assert!(!feishu("evilfeishucdn.com"));

        // Redirect policy shares the same matcher via host_matches.
        assert!(host_matches("LF6-UG-SIGN.FEISHUCDN.COM", "*.feishucdn.com"));
    }

    #[test]
    fn parses_installed_version_and_only_fully_installed_amd64() {
        assert_eq!(
            parse_installed_version_optional("ii \t1.2.3\tamd64", "amd64").unwrap(),
            Some("1.2.3".to_owned())
        );
        assert_eq!(
            parse_installed_version_optional("ii \t1.2.3\tarm64", "amd64").unwrap(),
            None
        );
        assert_eq!(
            parse_installed_version_optional("un \t1.2.3\tamd64", "amd64").unwrap(),
            None
        );
        assert!(parse_installed_version_optional("ii \t1.2.3\tamd64\textra", "amd64").is_err());
    }

    #[test]
    fn feed_entry_must_match_the_catalog_application() {
        let catalog = umanager_catalog::Catalog::load().unwrap();
        let code = catalog.by_application_id("vscode").unwrap().clone();
        let good = FeedApplicationEntry {
            package_name: "code".to_owned(),
            version: "1.2.3".to_owned(),
            architecture: "amd64".to_owned(),
            size: 100,
            sha256: "a".repeat(64),
            download_url: "https://packages.microsoft.com/repos/code/pool/main/c/code.deb".to_owned(),
            release_tag: None,
            asset_name: None,
            website_version: None,
            version_updated_at_unix_seconds: None,
            version_updated_at_source: None,
        };
        assert!(validate_feed_entry_for_app(&code, &good).is_ok());;

        let bad_host = FeedApplicationEntry {
            download_url: "https://evil.example/code.deb".to_owned(),
            ..good.clone()
        };
        assert!(validate_feed_entry_for_app(&code, &bad_host).is_err());

        let bad_arch = FeedApplicationEntry {
            architecture: "arm64".to_owned(),
            ..good.clone()
        };
        assert!(validate_feed_entry_for_app(&code, &bad_arch).is_err());
    }

    #[test]
    fn feed_plan_locks_the_feed_values_into_a_download_plan() {
        let catalog = umanager_catalog::Catalog::load().unwrap();
        let code = catalog.by_application_id("vscode").unwrap().clone();
        let entry = FeedApplicationEntry {
            package_name: "code".to_owned(),
            version: "1.2.3".to_owned(),
            architecture: "amd64".to_owned(),
            size: 42,
            sha256: "b".repeat(64),
            download_url: "https://packages.microsoft.com/repos/code/pool/main/c/code.deb".to_owned(),
            release_tag: None,
            asset_name: None,
            website_version: None,
            version_updated_at_unix_seconds: None,
            version_updated_at_source: None,
        };
        let plan = feed_plan(&code, Path::new("/tmp/cache"), &entry).unwrap();
        assert_eq!(plan.version, "1.2.3");
        assert_eq!(plan.expected_size, 42);
        assert_eq!(plan.expected_sha256.as_deref(), Some("b".repeat(64).as_str()));
        assert_eq!(plan.source_kind, SourceKind::OfficialRepository);
        assert!(plan.target_path.starts_with("/tmp/cache/downloads/"));
    }
}
