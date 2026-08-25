use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use umanager_catalog::{Application, Catalog, MetadataFeed};

/// Upper bound for the feed response; the feed is a tiny curated JSON document.
const MAX_FEED_BYTES: u64 = 1024 * 1024;
/// How long a successfully fetched feed is reused before it is refreshed.
const FEED_TTL: Duration = Duration::from_secs(15 * 60);
const FEED_SCHEMA_VERSION: u32 = 1;

/// The curated metadata feed published by the UManager project (e.g. GitHub Pages).
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub schema_version: u32,
    pub generated_at_unix_seconds: u64,
    #[serde(default)]
    pub applications: HashMap<String, FeedApplicationEntry>,
    #[serde(default)]
    pub self_update: Option<FeedApplicationEntry>,
    #[serde(default)]
    pub development_tools: HashMap<String, FeedToolEntry>,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedToolEntry {
    pub npm_package: String,
    pub version: String,
}

struct CachedFeed {
    fetched_at: Instant,
    feed: Feed,
}

static FEED_CACHE: OnceLock<Mutex<HashMap<String, CachedFeed>>> = OnceLock::new();

/// Whether a metadata feed is configured in the embedded software source.
pub fn config(catalog: &Catalog) -> Option<&MetadataFeed> {
    catalog.metadata_feed.as_ref()
}

/// Fetch and return the metadata feed, reusing a cached copy for up to `FEED_TTL`.
pub async fn load(catalog: &Catalog) -> Result<Feed, String> {
    let feed_config = config(catalog).ok_or_else(|| "软件源未配置元数据源".to_owned())?;
    let cache = FEED_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
        if let Some(entry) = guard.get(&feed_config.url)
            && entry.fetched_at.elapsed() < FEED_TTL
        {
            return Ok(entry.feed.clone());
        }
    }
    let feed = fetch(feed_config).await?;
    let mut guard = cache.lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
    guard.insert(
        feed_config.url.clone(),
        CachedFeed {
            fetched_at: Instant::now(),
            feed: feed.clone(),
        },
    );
    Ok(feed)
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

async fn fetch(feed_config: &MetadataFeed) -> Result<Feed, String> {
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
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取元数据源内容失败：{error}"))?;
    if body.len() as u64 > MAX_FEED_BYTES {
        return Err("元数据源响应大小异常".to_owned());
    }
    let feed: Feed = serde_json::from_str(&body)
        .map_err(|error| format!("元数据源格式无效：{error}"))?;
    validate(&feed)?;
    Ok(feed)
}

fn validate(feed: &Feed) -> Result<(), String> {
    if feed.schema_version != FEED_SCHEMA_VERSION {
        return Err("不支持的元数据源版本".to_owned());
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
    }
    Ok(())
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
    Ok(())
}
