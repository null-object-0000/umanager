use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;
use umanager_catalog::{Application, Catalog, MetadataFeed};

/// Upper bound for the feed response; the feed is a tiny curated JSON document.
const MAX_FEED_BYTES: u64 = 1024 * 1024;
/// How long a successfully fetched feed is reused before it is refreshed.
const FEED_TTL: Duration = Duration::from_secs(15 * 60);
/// How often the background refresher wakes up to check for a newer feed.
const FEED_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
const FEED_SCHEMA_VERSION: u32 = 2;
/// Path (relative to the app cache dir) of the persisted, signature-verified feed.
const FEED_CACHE_FILE: &str = "feed/feed-cache.json";

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
    /// Markdown release notes for the current version (e.g. a GitHub release
    /// body). Optional; always shipped through the signed feed.
    #[serde(default)]
    pub release_notes: Option<String>,
    /// HTTPS URL to the canonical release page / full changelog. Optional.
    #[serde(default)]
    pub release_notes_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedToolEntry {
    /// npm package this tool entry belongs to, for cross-checking at detection
    /// time. `None` for tools distributed outside npm (e.g. git/Python).
    #[serde(default)]
    pub npm_package: Option<String>,
    pub version: String,
    #[serde(default)]
    pub version_updated_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub version_updated_at_source: Option<VersionUpdatedAtSource>,
    /// Markdown release notes for the current version (e.g. the matching section
    /// of the project's CHANGELOG.md). Optional; always shipped via the signed feed.
    #[serde(default)]
    pub release_notes: Option<String>,
    /// HTTPS URL to the canonical changelog page. Optional.
    #[serde(default)]
    pub release_notes_url: Option<String>,
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
    /// Whether the feed currently in memory came from the local disk cache
    /// (offline / stale copy) rather than a fresh network fetch this session.
    pub serving_from_cache: bool,
}

struct CachedFeed {
    /// Unix time when this feed was actually fetched and verified from the
    /// network (not when it was read from disk into memory).
    fetched_at_unix_seconds: u64,
    feed: Feed,
}

/// Whether the in-memory feed is recent enough to skip a network refresh.
fn cached_feed_is_fresh(entry: &CachedFeed) -> bool {
    unix_timestamp_now().saturating_sub(entry.fetched_at_unix_seconds) < FEED_TTL.as_secs()
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
        serving_from_cache: false,
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

/// App cache directory, set once during startup. `None` means persistence and
/// background refresh are unavailable (the feed then behaves as before: network
/// only, in-memory TTL).
static CACHE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

fn cache_dir() -> Option<&'static Path> {
    CACHE_DIR.get().and_then(|value| value.as_deref())
}

fn cached_feed_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(FEED_CACHE_FILE)
}

/// Serializes network fetches (including background/manual refreshes) so at most
/// one metadata-feed request is in flight. Unlike `state_lock` (a `std` Mutex),
/// this can be held across `.await`.
fn fetch_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Everything a successful network fetch produced, including the exact signed
/// bytes and signature needed to persist a re-verifiable local copy.
struct FetchedFeed {
    feed: Feed,
    raw_json: String,
    signature_hex: String,
    signature_verified: bool,
}

/// On-disk representation of the last known-good feed. The raw `feed.json` text
/// and signature hex are preserved verbatim so the embedded Ed25519 public key
/// can re-verify the copy on every load; a corrupted or tampered cache is simply
/// discarded and re-fetched.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedFeed {
    fetched_at_unix_seconds: u64,
    generated_at_unix_seconds: u64,
    feed_json: String,
    signature_hex: String,
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

/// Initialize the metadata-feed subsystem once during app startup: remember the
/// cache directory and launch the periodic background refresher. Must be called
/// before any `load` so the feed can be persisted and refreshed.
pub fn initialize(app: &tauri::AppHandle) {
    let directory = app.path().app_cache_dir().ok();
    let _ = CACHE_DIR.set(directory);
    tauri::async_runtime::spawn(run_background_refresher());
}

/// Periodic refresher that keeps the persisted feed reasonably fresh. It runs in
/// the background so the UI never blocks on it; a failed refresh only updates the
/// status's `last_error`. Each tick fetches only when the persisted copy is
/// missing or stale, so a fresh cache never triggers a startup network hit.
async fn run_background_refresher() {
    loop {
        if should_refresh_now() {
            let _ = refresh_once(false).await;
        }
        tokio::time::sleep(FEED_REFRESH_INTERVAL).await;
    }
}

/// Whether the on-disk feed cache is missing or past its freshness TTL.
fn should_refresh_now() -> bool {
    let Some(cache_dir) = cache_dir() else {
        return true;
    };
    let Ok(bytes) = std::fs::read(cached_feed_path(cache_dir)) else {
        return true;
    };
    let Ok(persisted) = serde_json::from_slice::<PersistedFeed>(&bytes) else {
        return true;
    };
    unix_timestamp_now().saturating_sub(persisted.fetched_at_unix_seconds) >= FEED_TTL.as_secs()
}

/// Refresh the feed from the network, unless `force` is false and the in-memory
/// copy is still fresh enough. Serialized with every other fetch via `fetch_lock`.
pub async fn refresh_once(force: bool) -> Result<Feed, String> {
    let catalog = Catalog::load()?;
    let feed_config = config(&catalog).ok_or_else(|| "软件源未配置元数据源".to_owned())?;
    let _guard = fetch_lock().lock().await;
    if !force {
        let state = state_lock();
        let guard = state.lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
        if let Some(entry) = guard.cache.get(&feed_config.url)
            && cached_feed_is_fresh(entry)
        {
            return Ok(entry.feed.clone());
        }
    }
    fetch_and_store(feed_config).await
}

/// Fetch and return the metadata feed. Preference order: whatever is already in
/// memory this session (serving stale copies while a background refresh runs), the
/// persisted on-disk copy (re-verified against the embedded public key), and
/// finally the network.
pub async fn load(catalog: &Catalog) -> Result<Feed, String> {
    let feed_config = config(catalog).ok_or_else(|| "软件源未配置元数据源".to_owned())?;
    let feed_url = feed_config.url.clone();

    // 1. In-memory copy — the common fast path once the app is running. Serve it
    // as-is (stale-while-revalidate) and let a background refresh update it.
    {
        let state = state_lock();
        let guard = state.lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
        if let Some(entry) = guard.cache.get(&feed_url) {
            let stale = !cached_feed_is_fresh(entry);
            let feed = entry.feed.clone();
            drop(guard);
            if stale {
                tauri::async_runtime::spawn(async move {
                    let _ = refresh_once(false).await;
                });
            }
            return Ok(feed);
        }
    }

    // 2. Persisted copy — re-verify the signature, serve it immediately, and
    // refresh in the background if it is stale.
    if let Some(cache_dir) = cache_dir()
        && let Some((feed, fetched_at, extra)) = load_disk_feed(cache_dir)
    {
        let stale = unix_timestamp_now().saturating_sub(fetched_at) >= FEED_TTL.as_secs();
        {
            let state = state_lock();
            let mut guard = state.lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
            guard.status = FeedStatus {
                configured: true,
                url: Some(feed_url.clone()),
                signature_enforced: true,
                signature_verified: true,
                last_success_at_unix_seconds: Some(fetched_at),
                generated_at_unix_seconds: Some(feed.generated_at_unix_seconds),
                applications: feed.applications.len(),
                development_tools: feed.development_tools.len(),
                last_error: None,
                serving_from_cache: true,
            };
            guard.catalog_json = feed.catalog_json.clone();
            guard.catalog_signature = feed.catalog_signature.clone();
            guard.extra_applications = extra;
            guard.cache.insert(
                feed_url.clone(),
                CachedFeed {
                    fetched_at_unix_seconds: fetched_at,
                    feed: feed.clone(),
                },
            );
        }
        if stale {
            tauri::async_runtime::spawn(async move {
                let _ = refresh_once(false).await;
            });
        }
        return Ok(feed);
    }

    // 3. Network — serialized so concurrent startup requests don't double-fetch.
    let _guard = fetch_lock().lock().await;
    {
        let state = state_lock();
        let guard = state.lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
        if let Some(entry) = guard.cache.get(&feed_url) {
            return Ok(entry.feed.clone());
        }
    }
    fetch_and_store(feed_config).await
}

/// Fetch from the network, validate, persist to disk and update the shared state.
/// Also used by the background and manual refresh paths.
async fn fetch_and_store(feed_config: &MetadataFeed) -> Result<Feed, String> {
    match fetch(feed_config).await {
        Ok(fetched) => {
            let extra = parse_extra_applications(&fetched.feed)?;
            let network_time = unix_timestamp_now();
            if let Some(cache_dir) = cache_dir() {
                // Best-effort: a persistence failure must not fail the fetch —
                // the freshly verified feed is still usable in memory.
                let _ = persist_feed(cache_dir, &fetched, network_time).await;
            }
            let mut guard = state_lock().lock().map_err(|_| "元数据缓存锁失效".to_owned())?;
            guard.status = FeedStatus {
                configured: true,
                url: Some(feed_config.url.clone()),
                signature_enforced: true,
                signature_verified: fetched.signature_verified,
                last_success_at_unix_seconds: Some(network_time),
                generated_at_unix_seconds: Some(fetched.feed.generated_at_unix_seconds),
                applications: fetched.feed.applications.len(),
                development_tools: fetched.feed.development_tools.len(),
                last_error: None,
                serving_from_cache: false,
            };
            guard.catalog_json = fetched.feed.catalog_json.clone();
            guard.catalog_signature = fetched.feed.catalog_signature.clone();
            guard.extra_applications = extra;
            guard.cache.insert(
                feed_config.url.clone(),
                CachedFeed {
                    fetched_at_unix_seconds: network_time,
                    feed: fetched.feed.clone(),
                },
            );
            Ok(fetched.feed)
        }
        Err(error) => {
            if let Ok(mut guard) = state_lock().lock() {
                guard.status.last_error = Some(error.clone());
            }
            Err(error)
        }
    }
}

/// Read, re-verify and decode the persisted feed. Returns `None` if there is no
/// usable copy — a missing, corrupted, unverifiable or stale-schema cache is
/// silently ignored and the caller falls back to the network.
fn load_disk_feed(cache_dir: &Path) -> Option<(Feed, u64, Vec<Application>)> {
    let path = cached_feed_path(cache_dir);
    let bytes = std::fs::read(&path).ok()?;
    let persisted: PersistedFeed = serde_json::from_slice(&bytes).ok()?;
    verify_ed25519(persisted.feed_json.as_bytes(), persisted.signature_hex.trim()).ok()?;
    let feed: Feed = serde_json::from_str(&persisted.feed_json).ok()?;
    if validate(&feed).is_err() {
        return None;
    }
    if feed.generated_at_unix_seconds != persisted.generated_at_unix_seconds {
        return None;
    }
    let extra = parse_extra_applications(&feed).ok()?;
    Some((feed, persisted.fetched_at_unix_seconds, extra))
}

/// Atomically persist the just-fetched, already-verified feed. Failures here must
/// not fail the fetch itself, so the error is surfaced through `last_error` but
/// the caller keeps serving the in-memory copy.
async fn persist_feed(cache_dir: &Path, fetched: &FetchedFeed, network_time: u64) -> Result<(), String> {
    let persisted = PersistedFeed {
        fetched_at_unix_seconds: network_time,
        generated_at_unix_seconds: fetched.feed.generated_at_unix_seconds,
        feed_json: fetched.raw_json.clone(),
        signature_hex: fetched.signature_hex.clone(),
    };
    let bytes = serde_json::to_vec(&persisted).map_err(|error| format!("无法序列化元数据缓存：{error}"))?;
    let path = cached_feed_path(cache_dir);
    let directory = path.parent().ok_or_else(|| "元数据缓存路径无效".to_owned())?;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|error| format!("无法创建元数据缓存目录：{error}"))?;
    let temporary = directory.join("feed-cache.json.tmp");
    tokio::fs::write(&temporary, &bytes)
        .await
        .map_err(|error| format!("无法写入元数据缓存：{error}"))?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|error| format!("无法替换元数据缓存：{error}"))?;
    Ok(())
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

async fn fetch(feed_config: &MetadataFeed) -> Result<FetchedFeed, String> {
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

    let signature_hex = verify_feed_signature(&client, &feed_config.url, &hosts, &bytes).await?;
    let raw_json = String::from_utf8(bytes.to_vec())
        .map_err(|_| "元数据源内容不是有效的 UTF-8".to_owned())?;
    let feed: Feed = serde_json::from_str(&raw_json)
        .map_err(|error| format!("元数据源格式无效：{error}"))?;
    validate(&feed)?;
    Ok(FetchedFeed {
        feed,
        raw_json,
        signature_hex,
        signature_verified: true,
    })
}

async fn verify_feed_signature(
    client: &reqwest::Client,
    feed_url: &str,
    hosts: &[String],
    message: &[u8],
) -> Result<String, String> {
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
    let signature_hex = signature_hex.trim().to_owned();
    verify_ed25519(message, &signature_hex)?;
    Ok(signature_hex)
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
        if let Some(pkg) = &entry.npm_package {
            if pkg.is_empty() || pkg.contains('\0') {
                return Err(format!("{id}：npm 包名无效"));
            }
        }
        if entry.version.is_empty() || entry.version.contains('\0') {
            return Err(format!("{id}：版本无效"));
        }
        validate_version_updated_at(&entry.version_updated_at_unix_seconds, &entry.version_updated_at_source)
            .map_err(|error| format!("{id}：{error}"))?;
        validate_release_notes(&entry.release_notes, &entry.release_notes_url)
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
    validate_release_notes(&entry.release_notes, &entry.release_notes_url)?;
    Ok(())
}

/// Release notes (and their canonical URL) are display-only signed-feed data,
/// so they are allowed to be absent but must be structurally sound when
/// present: notes are plain UTF-8 text without NUL bytes, and the URL must be
/// HTTPS. The feed itself is capped at `MAX_FEED_BYTES`, so a per-entry length
/// bound is a defense-in-depth guard rather than the primary limit.
fn validate_release_notes(notes: &Option<String>, url: &Option<String>) -> Result<(), String> {
    if let Some(notes) = notes {
        if notes.contains('\0') {
            return Err("版本更新记录包含无效字符".to_owned());
        }
        if notes.len() > 200_000 {
            return Err("版本更新记录过长".to_owned());
        }
    }
    if let Some(url) = url
        && !url.starts_with("https://")
    {
        return Err("版本更新记录链接必须为 HTTPS".to_owned());
    }
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
