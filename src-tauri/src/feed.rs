use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use umanager_catalog::{Application, Catalog, MetadataFeed};

/// Upper bound for the feed response; the feed is a tiny curated JSON document.
const MAX_FEED_BYTES: u64 = 1024 * 1024;
/// How long a successfully fetched feed is reused before it is refreshed.
const FEED_TTL: Duration = Duration::from_secs(15 * 60);
const FEED_SCHEMA_VERSION: u32 = 2;

/// Embedded Ed25519 public key (raw 32 bytes, hex) that the feed must be signed
/// with. The matching private key lives only in the GitHub Actions secret
/// `FEED_SIGNING_KEY` and is never shipped with the application.
const FEED_PUBLIC_KEY_HEX: &str = "57d369d3e46b3243073b4535673ffa784dc760e0f14d6d25fb04940b69b0c8f9";

/// The curated metadata feed published by the UManager project (e.g. GitHub Pages).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub schema_version: u32,
    pub generated_at_unix_seconds: u64,
    #[serde(default)]
    pub applications: HashMap<String, FeedApplicationEntry>,
    /// Signed catalog of feed-added applications. `catalog_json` is the exact
    /// signed text (a JSON array of `Application`), `catalog_signature` is the
    /// Ed25519 signature over those bytes. The privileged helper verifies the
    /// same pair before accepting a feed-added application as allowlisted.
    #[serde(default)]
    pub catalog_json: Option<String>,
    #[serde(default)]
    pub catalog_signature: Option<String>,
    #[serde(default)]
    pub self_update: Option<FeedApplicationEntry>,
    #[serde(default)]
    pub development_tools: HashMap<String, FeedToolEntry>,
    /// Display-only software categories (grouping labels shown in the store).
    #[serde(default)]
    pub categories: Vec<FeedCategory>,
    /// Which applicationId / toolId belongs to which category id. Purely
    /// presentational — never consulted by the privileged helper or any
    /// authorization decision.
    #[serde(default)]
    pub category_assignments: FeedCategoryAssignments,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VersionUpdatedAtSource {
    Official,
    ServerModified,
    Observed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedApplicationEntry {
    pub package_name: String,
    pub version: String,
    pub architecture: String,
    pub size: u64,
    pub sha256: String,
    pub download_url: String,
    #[serde(default)]
    pub release_tag: Option<String>,
    #[serde(default)]
    pub asset_name: Option<String>,
    #[serde(default)]
    pub website_version: Option<String>,
    #[serde(default)]
    pub version_updated_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub version_updated_at_source: Option<VersionUpdatedAtSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedToolEntry {
    pub npm_package: String,
    pub version: String,
    #[serde(default)]
    pub version_updated_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub version_updated_at_source: Option<VersionUpdatedAtSource>,
}

/// A display-only software category served by the signed feed.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedCategory {
    pub id: String,
    pub label: String,
}

/// Category assignments keyed by application id / tool id. Values are category
/// ids (see `FeedCategory`). Purely presentational.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedCategoryAssignments {
    #[serde(default)]
    pub applications: HashMap<String, String>,
    #[serde(default)]
    pub development_tools: HashMap<String, String>,
}

/// Categories + assignments returned to the UI. The UI falls back to its
/// built-in mapping when this is unavailable (older feed or fetch failure).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCatalog {
    pub categories: Vec<FeedCategory>,
    pub assignments: FeedCategoryAssignments,
}

/// Human-readable snapshot of the last metadata-feed fetch, shown in Settings.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedStatus {
    pub configured: bool,
    pub url: Option<String>,
    pub signature_enforced: bool,
    pub signature_verified: bool,
    pub last_success_at_unix_seconds: Option<u64>,
    pub generated_at_unix_seconds: Option<u64>,
    pub applications: usize,
    pub development_tools: usize,
    pub last_error: Option<String>,
}

struct CachedFeed {
    fetched_at: Instant,
    feed: Feed,
}

struct FeedState {
    cache: HashMap<String, CachedFeed>,
    status: FeedStatus,
    catalog_json: Option<String>,
    catalog_signature: Option<String>,
    extra_applications: Vec<Application>,
}

fn initial_status(catalog: &Catalog) -> FeedStatus {
    let configured = catalog.metadata_feed.as_ref();
    FeedStatus {
        configured: configured.is_some(),
        url: configured.map(|value| value.url.clone()),
        signature_enforced: true,
        signature_verified: false,
        last_success_at_unix_seconds: None,
        generated_at_unix_seconds: None,
        applications: 0,
        development_tools: 0,
        last_error: None,
    }
}

static FEED_STATE: OnceLock<Mutex<FeedState>> = OnceLock::new();

fn fallback_catalog() -> Catalog {
    Catalog {
        schema_version: FEED_SCHEMA_VERSION,
        applications: Vec::new(),
        development_toolchains: Vec::new(),
        development_tools: Vec::new(),
        self_update: None,
        metadata_feed: None,
    }
}

fn state_lock() -> &'static Mutex<FeedState> {
    FEED_STATE.get_or_init(|| {
        let catalog = Catalog::load().unwrap_or_else(|_| fallback_catalog());
        Mutex::new(FeedState {
            cache: HashMap::new(),
            status: initial_status(&catalog),
            catalog_json: None,
            catalog_signature: None,
            extra_applications: Vec::new(),
        })
    })
}

/// Current status of the metadata feed (safe to clone and return to the UI).
pub fn status() -> FeedStatus {
    state_lock()
        .lock()
        .map(|guard| guard.status.clone())
        .unwrap_or_else(|_| initial_status(&fallback_catalog()))
}

/// Whether a metadata feed is configured in the embedded software source.
pub fn config(catalog: &Catalog) -> Option<&MetadataFeed> {
    catalog.metadata_feed.as_ref()
}

/// Fetch and return the metadata feed, reusing a cached copy for up to `FEED_TTL`.
pub async fn load(catalog: &Catalog) -> Result<Feed, String> {
    let feed_config = config(catalog).ok_or_else(|| "软件源未配置元数据源".to_owned())?;
    {
        let state = state_lock();
        let guard = state.lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
        if let Some(entry) = guard.cache.get(&feed_config.url)
            && entry.fetched_at.elapsed() < FEED_TTL
        {
            return Ok(entry.feed.clone());
        }
    }

    match fetch(feed_config).await {
        Ok((feed, signature_verified)) => {
            let extra = parse_extra_applications(&feed)?;
            let mut guard = state_lock().lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
            guard.status = FeedStatus {
                configured: true,
                url: Some(feed_config.url.clone()),
                signature_enforced: true,
                signature_verified,
                last_success_at_unix_seconds: Some(unix_timestamp_now()),
                generated_at_unix_seconds: Some(feed.generated_at_unix_seconds),
                applications: feed.applications.len(),
                development_tools: feed.development_tools.len(),
                last_error: None,
            };
            guard.catalog_json = feed.catalog_json.clone();
            guard.catalog_signature = feed.catalog_signature.clone();
            guard.extra_applications = extra;
            guard.cache.insert(
                feed_config.url.clone(),
                CachedFeed {
                    fetched_at: Instant::now(),
                    feed: feed.clone(),
                },
            );
            Ok(feed)
        }
        Err(error) => {
            if let Ok(mut guard) = state_lock().lock() {
                guard.status.last_error = Some(error.clone());
            }
            Err(error)
        }
    }
}

/// The full set of applications UManager manages: the compiled-in catalog plus
/// any feed-added applications. Feed-added entries only add new `applicationId`s;
/// they never replace a compiled-in definition.
pub async fn effective_applications() -> Result<Vec<Application>, String> {
    let catalog = Catalog::load()?;
    let mut applications = catalog.applications.clone();
    let feed = load(&catalog).await?;
    let extra = parse_extra_applications(&feed)?;
    for extra_app in extra {
        if !applications
            .iter()
            .any(|existing| existing.application_id == extra_app.application_id)
        {
            applications.push(extra_app);
        }
    }
    Ok(applications)
}

/// A `Catalog` whose `applications` also include any feed-added entries.
pub async fn effective_catalog() -> Result<Catalog, String> {
    let mut catalog = Catalog::load()?;
    catalog.applications = effective_applications().await?;
    Ok(catalog)
}

/// Raw signed catalog bytes for feed-added applications, for inclusion in an
/// immutable plan so the privileged helper can verify and authorize them.
pub fn catalog_auth() -> Option<(String, String)> {
    let guard = state_lock().lock().ok()?;
    Some((guard.catalog_json.clone()?, guard.catalog_signature.clone()?))
}

/// Display-only software categories from the signed feed. Returns `None` when
/// the feed is not configured, cannot be fetched, or carries no categories —
/// the UI then falls back to its built-in mapping.
pub async fn category_catalog() -> Option<CategoryCatalog> {
    let catalog = Catalog::load().ok()?;
    let feed = load(&catalog).await.ok()?;
    if feed.categories.is_empty() {
        return None;
    }
    Some(CategoryCatalog {
        categories: feed.categories,
        assignments: feed.category_assignments,
    })
}

fn parse_extra_applications(feed: &Feed) -> Result<Vec<Application>, String> {
    let (Some(json), Some(signature)) = (&feed.catalog_json, &feed.catalog_signature) else {
        return Ok(Vec::new());
    };
    verify_ed25519(json.as_bytes(), signature)?;
    serde_json::from_str(json)
        .map_err(|error| format!("元数据源目录格式无效：{error}"))
}

pub async fn entry_for(app: &Application) -> Result<Option<FeedApplicationEntry>, String> {
    let catalog = Catalog::load()?;
    Ok(load(&catalog).await?.applications.get(&app.application_id).cloned())
}

pub async fn tool_entry(tool_id: &str) -> Result<Option<FeedToolEntry>, String> {
    let catalog = Catalog::load()?;
    Ok(load(&catalog).await?.development_tools.get(tool_id).cloned())
}

pub async fn self_update_entry() -> Result<Option<FeedApplicationEntry>, String> {
    let catalog = Catalog::load()?;
    Ok(load(&catalog).await?.self_update.clone())
}

async fn fetch(feed_config: &MetadataFeed) -> Result<(Feed, bool), String> {
    let hosts = feed_config.hosts.clone();
    let client = crate::source_engine::restricted_client(&hosts, Duration::from_secs(20))?;

    let response = client
        .get(&feed_config.url)
        .send()
        .await
        .map_err(|error| format!("读取元数据源失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("元数据源返回错误：{error}"))?;
    if response.url().scheme() != "https"
        || !crate::source_engine::host_allowed(response.url().host_str(), &hosts)
    {
        return Err("元数据源重定向到未授权域名".to_owned());
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_FEED_BYTES)
    {
        return Err("元数据源响应大小异常".to_owned());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取元数据源内容失败：{error}"))?;
    if bytes.len() as u64 > MAX_FEED_BYTES {
        return Err("元数据源响应大小异常".to_owned());
    }

    let signature_verified = verify_feed_signature(&client, &feed_config.url, &hosts, &bytes).await?;
    let feed: Feed = serde_json::from_slice(&bytes)
        .map_err(|error| format!("元数据源格式无效：{error}"))?;
    validate(&feed)?;
    Ok((feed, signature_verified))
}

async fn verify_feed_signature(
    client: &reqwest::Client,
    feed_url: &str,
    hosts: &[String],
    message: &[u8],
) -> Result<bool, String> {
    let signature_url = format!("{feed_url}.sig");
    let response = client
        .get(&signature_url)
        .send()
        .await
        .map_err(|error| format!("读取元数据源签名失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("元数据源签名返回错误：{error}"))?;
    if response.url().scheme() != "https"
        || !crate::source_engine::host_allowed(response.url().host_str(), hosts)
    {
        return Err("元数据源签名重定向到未授权域名".to_owned());
    }
    let signature_hex = response
        .text()
        .await
        .map_err(|error| format!("读取元数据源签名内容失败：{error}"))?;
    verify_ed25519(message, signature_hex.trim())?;
    Ok(true)
}

fn verify_ed25519(message: &[u8], signature_hex: &str) -> Result<(), String> {
    let key_bytes = decode_hex_32(FEED_PUBLIC_KEY_HEX)?;
    let signature_bytes = decode_hex_64(signature_hex)?;
    let public_key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key_bytes);
    public_key
        .verify(message, &signature_bytes)
        .map_err(|_| "元数据源签名校验失败".to_owned())
}

fn decode_hex_32(input: &str) -> Result<[u8; 32], String> {
    let bytes = decode_hex(input)?;
    let mut out = [0_u8; 32];
    if bytes.len() != out.len() {
        return Err("元数据源公钥长度无效".to_owned());
    }
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_hex_64(input: &str) -> Result<[u8; 64], String> {
    let bytes = decode_hex(input)?;
    let mut out = [0_u8; 64];
    if bytes.len() != out.len() {
        return Err("元数据源签名长度无效".to_owned());
    }
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if input.len() % 2 != 0 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("十六进制签名格式无效".to_owned());
    }
    (0..input.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&input[index..index + 2], 16)
                .map_err(|_| "十六进制签名格式无效".to_owned())
        })
        .collect()
}

fn validate(feed: &Feed) -> Result<(), String> {
    if feed.schema_version != FEED_SCHEMA_VERSION {
        return Err("不支持的元数据源版本".to_owned());
    }
    if feed.catalog_json.is_some() != feed.catalog_signature.is_some() {
        return Err("元数据源目录字段不完整".to_owned());
    }
    for (id, entry) in &feed.applications {
        validate_application_entry(id, entry).map_err(|error| format!("{id}：{error}"))?;
    }
    if let Some(entry) = &feed.self_update {
        validate_application_entry("selfUpdate", entry)?;
    }
    for (id, entry) in &feed.development_tools {
        if entry.npm_package.is_empty() || entry.npm_package.contains('\0') {
            return Err(format!("{id}：npm 包名无效"));
        }
        if entry.version.is_empty() || entry.version.contains('\0') {
            return Err(format!("{id}：版本无效"));
        }
        validate_version_updated_at(&entry.version_updated_at_unix_seconds, &entry.version_updated_at_source)
            .map_err(|error| format!("{id}：{error}"))?;
    }
    Ok(())
}

fn validate_version_updated_at(
    time: &Option<u64>,
    source: &Option<VersionUpdatedAtSource>,
) -> Result<(), String> {
    match (time, source) {
        (None, None) => Ok(()),
        (Some(t), Some(_)) if *t > 0 => Ok(()),
        _ => Err("版本更新时间与来源必须同时有值且时间大于 0".to_owned()),
    }
}

fn validate_application_entry(_id: &str, entry: &FeedApplicationEntry) -> Result<(), String> {
    for (name, value) in [
        ("packageName", &entry.package_name),
        ("version", &entry.version),
        ("architecture", &entry.architecture),
    ] {
        if value.is_empty() || value.contains('\0') {
            return Err(format!("{name} 无效"));
        }
    }
    if entry.size == 0 {
        return Err("文件大小无效".to_owned());
    }
    if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("SHA-256 无效".to_owned());
    }
    if !entry.download_url.starts_with("https://") {
        return Err("下载地址必须为 HTTPS".to_owned());
    }
    validate_version_updated_at(
        &entry.version_updated_at_unix_seconds,
        &entry.version_updated_at_source,
    )?;
    Ok(())
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
    fn feed_parses_categories_and_assignments() {
        let json = r#"{
            "schemaVersion": 2,
            "generatedAtUnixSeconds": 1750000000,
            "applications": {},
            "categories": [
                { "id": "dev-tools", "label": "开发工具" },
                { "id": "ai-tools", "label": "AI 工具" }
            ],
            "categoryAssignments": {
                "applications": { "vscode": "dev-tools" },
                "developmentTools": { "codex": "ai-tools" }
            }
        }"#;
        let feed: Feed = serde_json::from_str(json).unwrap();
        assert_eq!(feed.categories.len(), 2);
        assert_eq!(feed.categories[0].label, "开发工具");
        assert_eq!(feed.category_assignments.applications["vscode"], "dev-tools");
        assert_eq!(feed.category_assignments.development_tools["codex"], "ai-tools");
    }

    #[test]
    fn feed_without_categories_defaults_to_empty() {
        let json = r#"{
            "schemaVersion": 2,
            "generatedAtUnixSeconds": 1750000000,
            "applications": {}
        }"#;
        let feed: Feed = serde_json::from_str(json).unwrap();
        assert!(feed.categories.is_empty());
        assert!(feed.category_assignments.applications.is_empty());
        assert!(feed.category_assignments.development_tools.is_empty());
    }
}
