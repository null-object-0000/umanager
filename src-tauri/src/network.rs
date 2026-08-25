use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};

const CONFIG_FILE_NAME: &str = "network.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkSettings {
    pub proxy_enabled: bool,
    pub proxy_url: String,
}

static NETWORK_SETTINGS: OnceLock<Mutex<NetworkSettings>> = OnceLock::new();

fn settings_lock() -> &'static Mutex<NetworkSettings> {
    NETWORK_SETTINGS.get_or_init(|| Mutex::new(NetworkSettings::default()))
}

/// Snapshot of the currently active network settings (safe to clone across threads).
pub fn current() -> NetworkSettings {
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

/// Validate, persist and activate new network settings.
pub fn update(app: &AppHandle, settings: NetworkSettings) -> Result<NetworkSettings, String> {
    let sanitized = sanitize(settings)?;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("无法确定 UManager 配置目录：{error}"))?;
    std::fs::create_dir_all(&dir).map_err(|error| format!("无法创建 UManager 配置目录：{error}"))?;
    let path = dir.join(CONFIG_FILE_NAME);
    let json = serde_json::to_string_pretty(&sanitized)
        .map_err(|error| format!("无法编码网络设置：{error}"))?;
    std::fs::write(&path, json).map_err(|error| format!("无法保存网络设置：{error}"))?;
    if let Ok(mut guard) = settings_lock().lock() {
        *guard = sanitized.clone();
    }
    Ok(sanitized)
}

/// Apply the configured proxy to a reqwest client builder, if one is enabled.
pub fn apply_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let settings = current();
    if !settings.proxy_enabled || settings.proxy_url.is_empty() {
        return builder;
    }
    match reqwest::Proxy::all(&settings.proxy_url) {
        Ok(proxy) => builder.proxy(proxy),
        Err(_) => builder,
    }
}

/// Proxy environment variables for child processes (nvm / rustup), if enabled.
pub fn proxy_environment() -> Vec<(&'static str, String)> {
    let settings = current();
    if !settings.proxy_enabled || settings.proxy_url.is_empty() {
        return Vec::new();
    }
    let url = settings.proxy_url.clone();
    vec![
        ("HTTP_PROXY", url.clone()),
        ("HTTPS_PROXY", url.clone()),
        ("ALL_PROXY", url.clone()),
        ("http_proxy", url.clone()),
        ("https_proxy", url.clone()),
        ("all_proxy", url),
    ]
}

fn load_from_file(path: &Path) -> Option<NetworkSettings> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn sanitize(mut settings: NetworkSettings) -> Result<NetworkSettings, String> {
    settings.proxy_url = settings.proxy_url.trim().to_owned();
    if settings.proxy_enabled {
        if settings.proxy_url.is_empty() {
            return Err("启用代理时必须填写代理地址".to_owned());
        }
        validate_proxy_url(&settings.proxy_url)?;
    }
    Ok(settings)
}

fn validate_proxy_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|error| format!("代理地址无效：{error}"))?;
    if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err("代理地址必须以 http://、https://、socks5:// 或 socks5h:// 开头".to_owned());
    }
    if parsed.host_str().is_none() {
        return Err("代理地址缺少主机名".to_owned());
    }
    Ok(())
}
