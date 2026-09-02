use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const CONFIG_FILE_NAME: &str = "llm.json";
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

const SYSTEM_PROMPT: &str = "你是一名软件更新日志翻译助手。把用户提供的英文 Markdown 更新日志翻译成简体中文。\
要求：1) 完整保留 Markdown 结构（标题、列表、代码块、行内代码、链接、表格）；\
2) 只翻译正文文字，不要翻译 URL、代码、命令、版本号、分支名、技术专有名词；\
3) 直接输出翻译后的 Markdown，不要任何额外说明或前后缀。";

/// Streaming delta emitted to the frontend while a translation is in flight.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmTranslateDelta {
    pub request_id: String,
    pub delta: String,
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

/// Stream a changelog translation, emitting `llm-translate-delta` events as
/// each piece of text arrives, and returning the full translated Markdown when
/// the stream finishes.
pub async fn translate_streaming(
    app: &AppHandle,
    request_id: &str,
    text: &str,
) -> Result<String, String> {
    let settings = current();
    let endpoint = validated_endpoint(&settings)?;
    let client = build_client()?;
    let mut response = authorized_request(&client, &endpoint, &settings)
        .body(request_body(&settings, text, true).to_string())
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
            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };
            let Some(delta) = value
                .pointer("/choices/0/delta/content")
                .and_then(|item| item.as_str())
            else {
                continue;
            };
            full.push_str(delta);
            let _ = app.emit(
                "llm-translate-delta",
                LlmTranslateDelta {
                    request_id: request_id.to_owned(),
                    delta: delta.to_owned(),
                },
            );
        }
    }
    let trimmed = full.trim();
    if trimmed.is_empty() {
        return Err("LLM 返回了空内容".to_owned());
    }
    Ok(trimmed.to_owned())
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

fn request_body(settings: &LlmSettings, text: &str, stream: bool) -> serde_json::Value {
    serde_json::json!({
        "model": settings.model.trim(),
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
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
        .body(request_body(settings, text, false).to_string())
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
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
}
