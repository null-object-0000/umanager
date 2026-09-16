use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

const CONFIG_FILE_NAME: &str = "llm.json";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// `llm-translate-delta` 的段落标签：同一次请求会同时产出一份「更新重点」归纳
/// 和一份完整译文，前端按此字段分别投递到两个位置。
const SECTION_SUMMARY: &str = "summary";
const SECTION_TRANSLATION: &str = "translation";

const SYSTEM_PROMPT: &str = "你是一名软件更新日志翻译助手。把用户提供的英文 Markdown 更新日志翻译成简体中文。\
要求：1) 完整保留 Markdown 结构（标题、列表、代码块、行内代码、链接、表格）；\
2) 只翻译正文文字，不要翻译 URL、代码、命令、版本号、分支名、技术专有名词；\
3) 直接输出翻译后的 Markdown，不要任何额外说明或前后缀。";

/// 归纳总结用的提示词：与翻译并行发起，只产出「本次更新重点」条目。
const SUMMARY_PROMPT: &str = "你是一名软件更新日志归纳助手。阅读用户提供的 Markdown 更新日志，\
用简体中文写出这个版本最值得用户知道的更新重点。\
要求：1) 只输出 3-6 条，每条一行、以「- 」开头，不要标题、不要开场白、不要结尾总结；\
2) 每条一句话说清「改了什么、修了什么、对用户有什么影响」，优先挑用户可感知的变化，忽略纯内部改动；\
3) 同类条目合并为一条（例如一串依赖升级或代码重构），不要逐条复述原文；\
4) 版本号、命令、产品名、代码标识符保持原样，不要翻译。";

/// Streaming delta emitted to the frontend while a translation is in flight.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTranslateDelta {
    pub request_id: String,
    pub delta: String,
    /// `summary`（更新重点）或 `translation`（完整译文）。
    pub section: String,
}

/// 一次「翻译 + 归纳」的最终结果，也是前端缓存展示所需的全部信息。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogTranslation {
    /// 归纳出的更新重点（Markdown 列表）。总结请求失败时为 `None`，不影响译文。
    pub summary: Option<String>,
    pub translation: String,
    /// 是否直接来自本地缓存（未发起 LLM 请求）。
    pub cached: bool,
    /// 产出该结果的模型名，仅用于展示「上次翻译」的来源。
    pub model: String,
    pub created_at_unix_seconds: u64,
}

/// User-configured LLM endpoint for changelog translation. All fields are
/// optional apart from `enabled`; validation enforces them only when enabled.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LlmSettings {
    pub enabled: bool,
    /// OpenAI-compatible API root, e.g. `https://api.deepseek.com/v1`.
    /// The request is sent to `{base_url}/chat/completions`.
    pub base_url: String,
    /// Optional bearer token. Empty for local servers (Ollama / vLLM).
    pub api_key: String,
    /// Model name, e.g. `deepseek-chat`, `gpt-4o-mini`.
    pub model: String,
}

static LLM_SETTINGS: OnceLock<Mutex<LlmSettings>> = OnceLock::new();

fn settings_lock() -> &'static Mutex<LlmSettings> {
    LLM_SETTINGS.get_or_init(|| Mutex::new(LlmSettings::default()))
}

// ---------------------------------------------------------------------------
// 翻译结果缓存
//
// 翻译一次要花用户的 token，而且更新日志本身不会变，所以结果按「原文内容」做
// 内容寻址落盘：下次打开同一个更新日志时先读缓存直接展示，只有用户主动点
// 「重新翻译」才重新请求 LLM。
// ---------------------------------------------------------------------------

/// 缓存格式（提示词 + 落盘字段）的版本。任一提示词或字段语义变化时必须递增，
/// 旧条目会因为版本不匹配被忽略，而不是展示已经过期的结果。
const CACHE_VERSION: u32 = 1;
const CACHE_FILE_NAME: &str = "changelog-translations.json";
/// 缓存条目上限，超出后按写入时间淘汰最旧的，避免文件随浏览过的更新日志无限增长。
const MAX_CACHE_ENTRIES: usize = 64;
/// 缓存总体积上限。条数上限挡不住「一份超长更新日志」把文件撑大，所以再压一层；
/// 淘汰时始终保留最新写入的那一条。
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;

/// 一条落盘结果。字段名即文件格式的一部分，改动需同步 `CACHE_VERSION`。
/// 缺失字段一律回落到 `Default`（`version` = 0），因此截断或旧格式的文件不会
/// 被误当成有效缓存。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct CachedTranslation {
    /// 原文的 SHA-256（十六进制）。与 map 键重复，但让文件可自解释、便于排查。
    source_sha256: String,
    /// 写入时的 `CACHE_VERSION`。
    version: u32,
    model: String,
    summary: Option<String>,
    translation: String,
    created_at_unix_seconds: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct TranslationCache {
    #[serde(default)]
    entries: BTreeMap<String, CachedTranslation>,
}

/// 缓存所在目录（`app_data_dir`，与配置分开：译文不是密钥，但要跨重启保留）。
static CACHE_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
/// 串行化缓存文件的读改写，避免两个更新日志同时翻译时互相覆盖。
static CACHE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn cache_lock() -> &'static Mutex<()> {
    CACHE_LOCK.get_or_init(|| Mutex::new(()))
}

fn cache_path() -> Option<PathBuf> {
    CACHE_DIR
        .get()
        .and_then(|dir| dir.as_ref())
        .map(|dir| dir.join(CACHE_FILE_NAME))
}

/// 内容寻址键：同一份更新日志文本恒定映射到同一条缓存。
fn cache_key(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// 读取缓存文件；文件缺失、损坏或格式不认识时都视为「没有缓存」。
fn read_cache(path: &Path) -> TranslationCache {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<TranslationCache>(&bytes).ok())
        .unwrap_or_default()
}

/// 查一条缓存。返回 `None` 表示没有命中（含版本过期、内容为空等失效情况）。
fn lookup_cached(path: &Path, text: &str) -> Option<CachedTranslation> {
    let _guard = cache_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = read_cache(path).entries.remove(&cache_key(text))?;
    let usable = entry.version == CACHE_VERSION && !entry.translation.trim().is_empty();
    usable.then_some(entry)
}

/// 写入一条缓存并落盘（同步、小文件）。写失败不致命：结果照常返回给前端，
/// 只是下次需要重新翻译。
fn store_cached(path: &Path, text: &str, entry: CachedTranslation) -> std::io::Result<()> {
    let _guard = cache_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut cache = read_cache(path);
    cache.entries.insert(cache_key(text), entry);
    evict_oldest(&mut cache);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(&cache)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    write_private(path, &json)
}

fn evict_oldest(cache: &mut TranslationCache) {
    // 先按条数淘汰，再按总体积淘汰。
    while cache.entries.len() > MAX_CACHE_ENTRIES && remove_oldest(cache) {}
    while cache.entries.len() > 1 && entries_bytes(cache) > MAX_CACHE_BYTES && remove_oldest(cache) {}
}

/// 按写入时间淘汰一条最旧的；返回是否真的删掉了一条。
fn remove_oldest(cache: &mut TranslationCache) -> bool {
    let oldest = cache
        .entries
        .iter()
        .min_by_key(|(_, entry)| entry.created_at_unix_seconds)
        .map(|(key, _)| key.clone());
    match oldest {
        Some(key) => cache.entries.remove(&key).is_some(),
        None => false,
    }
}

fn entries_bytes(cache: &TranslationCache) -> usize {
    cache
        .entries
        .values()
        .map(|entry| entry.translation.len() + entry.summary.as_deref().map_or(0, str::len))
        .sum()
}

/// Snapshot of the active LLM settings (safe to clone across threads).
pub fn current() -> LlmSettings {
    settings_lock()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// Load persisted settings into the process-wide state. Called once during setup.
pub fn initialize(app: &AppHandle) {
    let loaded = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| load_from_file(&dir.join(CONFIG_FILE_NAME)))
        .flatten()
        .unwrap_or_default();
    if let Ok(mut guard) = settings_lock().lock() {
        *guard = loaded;
    }
    let _ = CACHE_DIR.set(app.path().app_data_dir().ok());
}

/// Validate, persist (with 0600 permissions on Unix) and activate new settings.
pub fn update(app: &AppHandle, settings: LlmSettings) -> Result<LlmSettings, String> {
    let sanitized = sanitize(settings)?;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法确定 UManager 配置目录：{error}"))?;
    std::fs::create_dir_all(&dir).map_err(|error| format!("无法创建 UManager 配置目录：{error}"))?;
    let path = dir.join(CONFIG_FILE_NAME);
    let json = serde_json::to_string_pretty(&sanitized)
        .map_err(|error| format!("无法编码 LLM 设置：{error}"))?;
    write_private(&path, json.as_bytes())
        .map_err(|error| format!("无法保存 LLM 设置：{error}"))?;
    if let Ok(mut guard) = settings_lock().lock() {
        *guard = sanitized.clone();
    }
    Ok(sanitized)
}

/// Verify the configured endpoint with a one-shot trivial translation. Accepts
/// an optional settings override so the settings form can test unsaved values.
pub async fn test_connection(settings: Option<LlmSettings>) -> Result<String, String> {
    let owned;
    let settings = match settings {
        Some(settings) => {
            owned = settings;
            &owned
        }
        None => &current(),
    };
    translate_with("Hello", settings).await
}

/// 只读缓存查询：不发任何 LLM 请求。前端在渲染更新日志时先问一次，命中就直接
/// 展示上次的译文与更新重点。
pub fn cached(text: &str) -> Option<ChangelogTranslation> {
    cached_at(&cache_path()?, text)
}

fn cached_at(path: &Path, text: &str) -> Option<ChangelogTranslation> {
    lookup_cached(path, text).map(|entry| ChangelogTranslation {
        summary: entry.summary,
        translation: entry.translation,
        cached: true,
        model: entry.model,
        created_at_unix_seconds: entry.created_at_unix_seconds,
    })
}

/// 流式增量的投递方式。抽成闭包后核心流程不再强依赖 `AppHandle`，单测可以直接
/// 注入一个收集器 + 本地假的 LLM 服务。
type Emit = std::sync::Arc<dyn Fn(LlmTranslateDelta) + Send + Sync>;

/// 翻译一份更新日志并同时归纳更新重点：译文以流式事件回传，总结与翻译并行请求
/// （总结短，通常先于译文完成），两者一起落盘复用。
///
/// `force` = false 时先查本地缓存：命中则完全不请求 LLM，直接返回上次结果。
/// 总结请求失败不算整体失败 —— 译文照常返回，只是没有「更新重点」。
pub async fn translate_streaming(
    app: &AppHandle,
    request_id: &str,
    text: &str,
    force: bool,
) -> Result<ChangelogTranslation, String> {
    let emit: Emit = {
        let app = app.clone();
        std::sync::Arc::new(move |delta: LlmTranslateDelta| {
            let _ = app.emit("llm-translate-delta", delta);
        })
    };
    let settings = current();
    translate_and_store(&settings, cache_path().as_deref(), request_id, text, force, &emit).await
}

async fn translate_and_store(
    settings: &LlmSettings,
    cache: Option<&Path>,
    request_id: &str,
    text: &str,
    force: bool,
    emit: &Emit,
) -> Result<ChangelogTranslation, String> {
    // 先校验配置，配置有问题时不要在缓存里留下任何东西。
    validated_endpoint(settings)?;
    if !force
        && let Some(hit) = cache.and_then(|path| cached_at(path, text))
    {
        return Ok(hit);
    }

    let summary_task = {
        let settings = settings.clone();
        let emit = emit.clone();
        let request_id = request_id.to_owned();
        let text = text.to_owned();
        tauri::async_runtime::spawn(async move {
            stream_completion(&settings, &emit, &request_id, SUMMARY_PROMPT, &text, SECTION_SUMMARY)
                .await
        })
    };

    let translation = match stream_completion(
        settings,
        emit,
        request_id,
        SYSTEM_PROMPT,
        text,
        SECTION_TRANSLATION,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            // 译文失败就没有结果可展示，别让总结请求白跑（它的 delta 已无人接收）。
            summary_task.abort();
            return Err(error);
        }
    };
    let summary = match summary_task.await {
        Ok(Ok(value)) => Some(value).filter(|value| !value.trim().is_empty()),
        _ => None,
    };

    let result = ChangelogTranslation {
        summary,
        translation,
        cached: false,
        model: settings.model.trim().to_owned(),
        created_at_unix_seconds: unix_timestamp_now(),
    };
    if let Some(path) = cache {
        let entry = CachedTranslation {
            source_sha256: cache_key(text),
            version: CACHE_VERSION,
            model: result.model.clone(),
            summary: result.summary.clone(),
            translation: result.translation.clone(),
            created_at_unix_seconds: result.created_at_unix_seconds,
        };
        // 落盘是尽力而为：写不进去也不该让用户看不到这次翻译结果。
        let _ = store_cached(path, text, entry);
    }
    Ok(result)
}

/// 用给定提示词向 LLM 发起一次流式请求，把每个增量按 `section` 标签投递出去，
/// 并在结束时返回完整文本。
async fn stream_completion(
    settings: &LlmSettings,
    emit: &Emit,
    request_id: &str,
    system_prompt: &str,
    text: &str,
    section: &str,
) -> Result<String, String> {
    let endpoint = validated_endpoint(settings)?;
    let client = build_client()?;
    let mut response = authorized_request(&client, &endpoint, settings)
        .body(request_body(settings, system_prompt, text, true).to_string())
        .send()
        .await
        .map_err(|error| format!("LLM 请求失败：{error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(read_llm_error(response, status).await);
    }

    let mut full = String::new();
    let mut buffer = String::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| format!("读取 LLM 流式响应失败：{error}"))?;
        let Some(chunk) = chunk else { break };
        if full.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("LLM 响应过大".to_owned());
        }
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        // Parse server-sent events: one `data: {...}` JSON per line.
        while let Some(newline) = buffer.find('\n') {
            let line = buffer[..newline].trim_end_matches('\r').to_owned();
            buffer.drain(..=newline);
            let Some(delta) = sse_delta(&line) else { continue };
            full.push_str(&delta);
            emit(LlmTranslateDelta {
                request_id: request_id.to_owned(),
                delta,
                section: section.to_owned(),
            });
        }
    }
    let trimmed = full.trim();
    if trimmed.is_empty() {
        return Err("LLM 返回了空内容".to_owned());
    }
    Ok(trimmed.to_owned())
}

/// 从一行 SSE 文本里取出增量内容。非 `data:` 行（心跳 / 注释）、`[DONE]` 结束标记、
/// 无法解析的 JSON，以及没有正文的增量都返回 `None`。
fn sse_delta(line: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let content = value
        .pointer("/choices/0/delta/content")
        .and_then(|item| item.as_str())?;
    (!content.is_empty()).then(|| content.to_owned())
}

fn validated_endpoint(settings: &LlmSettings) -> Result<String, String> {
    if !settings.enabled {
        return Err("尚未启用 LLM 翻译，请先到“设置 → LLM 翻译”配置服务".to_owned());
    }
    let base = settings.base_url.trim();
    if base.is_empty() || settings.model.trim().is_empty() {
        return Err("LLM 翻译配置不完整：请填写服务地址与模型名称".to_owned());
    }
    chat_completions_url(base)
}

fn request_body(
    settings: &LlmSettings,
    system_prompt: &str,
    text: &str,
    stream: bool,
) -> serde_json::Value {
    serde_json::json!({
        "model": settings.model.trim(),
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": text }
        ],
        "temperature": 0.2,
        "stream": stream
    })
}

fn authorized_request(
    client: &reqwest::Client,
    endpoint: &str,
    settings: &LlmSettings,
) -> reqwest::RequestBuilder {
    let mut request = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    if !settings.api_key.trim().is_empty() {
        request = request.header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", settings.api_key.trim()),
        );
    }
    request
}

async fn read_llm_error(response: reqwest::Response, status: reqwest::StatusCode) -> String {
    let message = response
        .bytes()
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(|item| item.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "未知错误".to_owned());
    format!("LLM 服务返回错误（HTTP {}）：{message}", status.as_u16())
}

async fn translate_with(text: &str, settings: &LlmSettings) -> Result<String, String> {
    let endpoint = validated_endpoint(settings)?;
    let client = build_client()?;
    let response = authorized_request(&client, &endpoint, settings)
        .body(request_body(settings, SYSTEM_PROMPT, text, false).to_string())
        .send()
        .await
        .map_err(|error| format!("LLM 请求失败：{error}"))?;
    let status = response.status();
    if let Some(length) = response.content_length()
        && length > MAX_RESPONSE_BYTES as u64
    {
        return Err("LLM 响应过大".to_owned());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 LLM 响应失败：{error}"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("LLM 响应过大".to_owned());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "LLM 响应不是有效的 JSON".to_owned())?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(|item| item.as_str())
            .unwrap_or("未知错误");
        return Err(format!("LLM 服务返回错误（HTTP {}）：{message}", status.as_u16()));
    }
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|item| item.as_str())
        .ok_or_else(|| "LLM 响应缺少内容".to_owned())?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("LLM 返回了空内容".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn chat_completions_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim_end_matches('/');
    let parsed = reqwest::Url::parse(trimmed).map_err(|error| format!("LLM 服务地址无效：{error}"))?;
    ensure_safe_endpoint(&parsed)?;
    if trimmed.ends_with("/chat/completions") {
        Ok(trimmed.to_owned())
    } else {
        Ok(format!("{trimmed}/chat/completions"))
    }
}

/// Only `https://` endpoints are trusted in general. `http://` is allowed only
/// for loopback hosts (Ollama / vLLM running locally), where the API key — if
/// any — never leaves the machine over the network.
fn ensure_safe_endpoint(url: &reqwest::Url) -> Result<(), String> {
    let scheme = url.scheme();
    if scheme == "https" {
        return Ok(());
    }
    if scheme == "http"
        && url
            .host_str()
            .is_some_and(|host| host == "localhost" || host == "::1" || host.starts_with("127."))
    {
        return Ok(());
    }
    Err("LLM 服务地址必须为 https://（本地服务可使用 http://localhost 或 http://127.0.0.1）".to_owned())
}

fn build_client() -> Result<reqwest::Client, String> {
    crate::network::apply_proxy(reqwest::Client::builder())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("请求重定向次数超过限制");
            }
            match reqwest::Url::parse(attempt.url().as_str()) {
                Ok(url) if ensure_safe_endpoint(&url).is_ok() => attempt.follow(),
                _ => attempt.error("请求重定向到不安全地址"),
            }
        }))
        .user_agent("UManager/0.1")
        .build()
        .map_err(|error| format!("无法创建 LLM 请求客户端：{error}"))
}

fn load_from_file(path: &Path) -> Option<LlmSettings> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn sanitize(mut settings: LlmSettings) -> Result<LlmSettings, String> {
    settings.base_url = settings.base_url.trim().to_owned();
    settings.api_key = settings.api_key.trim().to_owned();
    settings.model = settings.model.trim().to_owned();
    if settings.enabled {
        if settings.base_url.is_empty() {
            return Err("启用 LLM 翻译时必须填写服务地址".to_owned());
        }
        if settings.model.is_empty() {
            return Err("启用 LLM 翻译时必须填写模型名称".to_owned());
        }
        // Validate early so a typo is caught at save time, not on first use.
        chat_completions_url(&settings.base_url)?;
    }
    Ok(settings)
}

/// Write a config file with owner-only permissions on Unix (the LLM API key is
/// a secret and must not be world-readable).
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_endpoints_accept_https_and_loopback_http() {
        assert!(ensure_safe_endpoint(&reqwest::Url::parse("https://api.deepseek.com/v1").unwrap()).is_ok());
        assert!(ensure_safe_endpoint(&reqwest::Url::parse("http://localhost:11434/v1").unwrap()).is_ok());
        assert!(ensure_safe_endpoint(&reqwest::Url::parse("http://127.0.0.1:8000/v1").unwrap()).is_ok());
        assert!(ensure_safe_endpoint(&reqwest::Url::parse("http://example.com/v1").unwrap()).is_err());
        assert!(ensure_safe_endpoint(&reqwest::Url::parse("ftp://example.com").unwrap()).is_err());
    }

    #[test]
    fn chat_completions_url_appends_path_once() {
        assert_eq!(
            chat_completions_url("https://api.deepseek.com/v1/").unwrap(),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("https://api.deepseek.com/v1/chat/completions").unwrap(),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn sanitize_requires_endpoint_and_model_when_enabled() {
        let base = LlmSettings {
            enabled: true,
            base_url: "https://api.deepseek.com/v1".to_owned(),
            api_key: "sk-test".to_owned(),
            model: "deepseek-chat".to_owned(),
        };
        assert!(sanitize(base.clone()).is_ok());

        let missing_model = LlmSettings { model: "".to_owned(), ..base.clone() };
        assert!(sanitize(missing_model).is_err());

        let missing_url = LlmSettings { base_url: "".to_owned(), ..base.clone() };
        assert!(sanitize(missing_url).is_err());

        let disabled = LlmSettings { enabled: false, ..base };
        assert!(sanitize(disabled).is_ok());
    }

    /// 每个用例独占一个临时文件，避免并行测试互相覆盖。
    fn temp_cache_path() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = format!(
            "umanager-translation-{}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        );
        std::env::temp_dir().join(unique).join(CACHE_FILE_NAME)
    }

    fn entry(translation: &str, created_at_unix_seconds: u64) -> CachedTranslation {
        CachedTranslation {
            source_sha256: String::new(),
            version: CACHE_VERSION,
            model: "deepseek-chat".to_owned(),
            summary: Some("- 更新重点".to_owned()),
            translation: translation.to_owned(),
            created_at_unix_seconds,
        }
    }

    #[test]
    fn cache_key_is_content_addressed() {
        let key = cache_key("## v1.2.0\n- fix crash");
        assert_eq!(key, cache_key("## v1.2.0\n- fix crash"));
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(key, cache_key("## v1.2.1\n- fix crash"));
    }

    #[test]
    fn cached_translation_round_trips_through_the_file() {
        let path = temp_cache_path();
        let text = "## v1.2.0\n- fix crash on startup";
        assert!(lookup_cached(&path, text).is_none());

        store_cached(&path, text, entry("## v1.2.0\n- 修复启动崩溃", 1_700_000_000)).unwrap();

        let hit = lookup_cached(&path, text).expect("cache hit");
        assert_eq!(hit.translation, "## v1.2.0\n- 修复启动崩溃");
        assert_eq!(hit.summary.as_deref(), Some("- 更新重点"));
        assert_eq!(hit.created_at_unix_seconds, 1_700_000_000);
        assert!(lookup_cached(&path, "别的更新日志").is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cached_translation_from_another_prompt_version_is_ignored() {
        let path = temp_cache_path();
        let text = "## v1.2.0";
        let stale = CachedTranslation { version: CACHE_VERSION + 1, ..entry("旧译文", 1) };
        store_cached(&path, text, stale).unwrap();

        assert!(lookup_cached(&path, text).is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn empty_or_corrupt_cache_files_are_treated_as_a_miss() {
        let path = temp_cache_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ this is not json").unwrap();
        assert!(lookup_cached(&path, "## v1.2.0").is_none());

        // 损坏文件不应阻止后续写入。
        store_cached(&path, "## v1.2.0", entry("译文", 2)).unwrap();
        assert!(lookup_cached(&path, "## v1.2.0").is_some());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cache_keeps_the_newest_entries_within_the_limit() {
        let path = temp_cache_path();
        for index in 0..(MAX_CACHE_ENTRIES + 5) {
            store_cached(&path, &format!("changelog #{index}"), entry("译文", index as u64)).unwrap();
        }

        let stored = read_cache(&path);
        assert_eq!(stored.entries.len(), MAX_CACHE_ENTRIES);
        // 最旧的 5 条被淘汰，最新的一条仍在。
        assert!(lookup_cached(&path, "changelog #0").is_none());
        assert!(lookup_cached(&path, "changelog #4").is_none());
        assert!(lookup_cached(&path, &format!("changelog #{}", MAX_CACHE_ENTRIES + 4)).is_some());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cache_evicts_oldest_entries_beyond_the_size_limit() {
        let path = temp_cache_path();
        // 每条译文约占 1.5 MiB（中文 3 字节/字），两条就超过 2 MiB 的总体积上限。
        let huge = "译".repeat(MAX_CACHE_BYTES / 4);
        for index in 0..6 {
            store_cached(&path, &format!("changelog #{index}"), entry(&huge, index as u64)).unwrap();
        }

        let stored = read_cache(&path);
        assert_eq!(stored.entries.len(), 1, "只保留能装下的最新一条");
        assert!(entries_bytes(&stored) <= MAX_CACHE_BYTES);
        assert!(lookup_cached(&path, "changelog #5").is_some());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn sse_delta_reads_content_and_skips_noise_lines() {
        assert_eq!(
            sse_delta(r#"data: {"choices":[{"delta":{"content":"v1.2.0 released"}}]}"#).as_deref(),
            Some("v1.2.0 released")
        );
        // 空行 / 心跳 / 注释 / 结束标记 / 坏 JSON 都不产生增量。
        assert_eq!(sse_delta(""), None);
        assert_eq!(sse_delta("data: "), None);
        assert_eq!(sse_delta("data: [DONE]"), None);
        assert_eq!(sse_delta(": keep-alive"), None);
        assert_eq!(sse_delta("data: {not json}"), None);
        // 没有 content 字段（例如只有 role 的首个增量）也跳过。
        assert_eq!(sse_delta(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#), None);
        assert_eq!(sse_delta(r#"data: {"choices":[{"delta":{"content":""}}]}"#), None);
    }

    #[test]
    fn request_body_carries_the_section_prompt_and_stream_flag() {
        let settings = LlmSettings {
            enabled: true,
            base_url: "https://api.deepseek.com/v1".to_owned(),
            api_key: "sk-test".to_owned(),
            model: "deepseek-chat".to_owned(),
        };
        let body = request_body(&settings, SUMMARY_PROMPT, "release notes", true);

        assert_eq!(body.pointer("/model").and_then(|item| item.as_str()), Some("deepseek-chat"));
        assert_eq!(body.pointer("/stream").and_then(|item| item.as_bool()), Some(true));
        assert_eq!(
            body.pointer("/messages/0/content").and_then(|item| item.as_str()),
            Some(SUMMARY_PROMPT)
        );
        assert_eq!(
            body.pointer("/messages/1/content").and_then(|item| item.as_str()),
            Some("release notes")
        );
        assert_ne!(
            body.pointer("/messages/0/content").and_then(|item| item.as_str()),
            Some(SYSTEM_PROMPT)
        );
    }

    // ---------------------------------------------------------------------
    // 「翻译 + 归纳 + 缓存」的端到端测试：本地起一个假的 OpenAI 兼容服务，
    // 按请求里的 system 提示词区分总结/翻译，各自回一段 SSE 流。
    // ---------------------------------------------------------------------

    /// 极简假服务，返回 `(base_url, 命中次数)`。`summary_fails` 为真时对总结请求
    /// 返回 HTTP 500，用于验证「总结失败不影响译文」。
    fn spawn_mock_llm(summary_fails: bool) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock llm");
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();

        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else { break };
                let counter = counter.clone();
                std::thread::spawn(move || {
                    let Ok(mut reader) = stream.try_clone().map(BufReader::new) else { return };
                    // 读完请求头拿 Content-Length，再按长度读完 JSON body。
                    let mut length = 0usize;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let header = line.trim_end();
                        if header.is_empty() {
                            break;
                        }
                        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0u8; length];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    counter.fetch_add(1, Ordering::SeqCst);

                    let is_summary = String::from_utf8_lossy(&body).contains(SUMMARY_PROMPT);
                    let mut stream = stream;
                    let response = match (is_summary, summary_fails) {
                        (true, true) => "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"error\":{\"message\":\"summary down\"}}".to_owned(),
                        (true, false) => sse_response(&["- 修复", "启动崩溃"]),
                        (false, _) => sse_response(&["修复了", "启动崩溃"]),
                    };
                    let _ = stream.write_all(response.as_bytes());
                });
            }
        });

        (format!("http://127.0.0.1:{port}/v1"), hits)
    }

    fn sse_response(chunks: &[&str]) -> String {
        let mut response = String::from(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        );
        for chunk in chunks {
            let content = serde_json::to_string(chunk).unwrap();
            response.push_str(&format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":{content}}}}}]}}\n\n"
            ));
        }
        response.push_str("data: [DONE]\n\n");
        response
    }

    fn mock_settings(base_url: String) -> LlmSettings {
        LlmSettings {
            enabled: true,
            base_url,
            api_key: "sk-test".to_owned(),
            model: "mock-model".to_owned(),
        }
    }

    #[test]
    fn translate_and_store_emits_both_sections_and_reuses_the_cache() {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;

        let path = temp_cache_path();
        let (base_url, hits) = spawn_mock_llm(false);
        let settings = mock_settings(base_url);
        let emitted = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let emit: Emit = {
            let sink = emitted.clone();
            Arc::new(move |delta: LlmTranslateDelta| {
                sink.lock().unwrap().push((delta.section, delta.delta));
            })
        };

        let first = tauri::async_runtime::block_on(translate_and_store(
            &settings,
            Some(&path),
            "req-1",
            "release notes",
            false,
            &emit,
        ))
        .expect("translation succeeds");
        assert_eq!(first.translation, "修复了启动崩溃");
        assert_eq!(first.summary.as_deref(), Some("- 修复启动崩溃"));
        assert!(!first.cached);
        assert_eq!(first.model, "mock-model");
        // 总结与译文各一个请求，并行发出。
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        let sections: Vec<String> = emitted
            .lock()
            .unwrap()
            .iter()
            .map(|(section, _)| section.clone())
            .collect();
        assert!(sections.contains(&SECTION_SUMMARY.to_owned()));
        assert!(sections.contains(&SECTION_TRANSLATION.to_owned()));
        assert_eq!(sections.len(), 4);

        // 第二次打开同一份更新日志：命中缓存，不再产生任何 LLM 请求。
        let second = tauri::async_runtime::block_on(translate_and_store(
            &settings,
            Some(&path),
            "req-2",
            "release notes",
            false,
            &emit,
        ))
        .expect("cache hit");
        assert!(second.cached);
        assert_eq!(second.translation, first.translation);
        assert_eq!(second.summary, first.summary);
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        // 「重新翻译」绕过缓存。
        let third = tauri::async_runtime::block_on(translate_and_store(
            &settings,
            Some(&path),
            "req-3",
            "release notes",
            true,
            &emit,
        ))
        .expect("forced translation");
        assert!(!third.cached);
        assert_eq!(hits.load(Ordering::SeqCst), 4);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn summary_failure_still_returns_and_caches_the_translation() {
        use std::sync::Arc;

        let path = temp_cache_path();
        let (base_url, hits) = spawn_mock_llm(true);
        let settings = mock_settings(base_url);
        let emit: Emit = Arc::new(|_: LlmTranslateDelta| {});

        let first = tauri::async_runtime::block_on(translate_and_store(
            &settings,
            Some(&path),
            "req-1",
            "release notes",
            false,
            &emit,
        ))
        .expect("translation succeeds without a summary");
        assert_eq!(first.translation, "修复了启动崩溃");
        assert_eq!(first.summary, None);

        // 译文仍然落盘：下次打开直接展示译文，只是没有「更新重点」。
        let second = tauri::async_runtime::block_on(translate_and_store(
            &settings,
            Some(&path),
            "req-2",
            "release notes",
            false,
            &emit,
        ))
        .expect("cache hit");
        assert!(second.cached);
        assert_eq!(second.translation, "修复了启动崩溃");
        assert_eq!(second.summary, None);
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "只有第一次的两个请求"
        );

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
