use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

/// Fetches (or reuses a cached copy of) the feed-published app icon and returns
/// it as a `data:image/png;base64,<...>` URL the webview can render directly.
///
/// The icon is downloaded over HTTPS from the metadata-feed host allowlist,
/// verified against `icon_sha256`, then written to the cache keyed by
/// `<appId>-<sha256前16>.png` so a cache hit can never serve wrong bytes.
pub async fn fetch_app_icon(
    cache_dir: &Path,
    app_id: &str,
    icon_url: &str,
    icon_sha256: &str,
    hosts: &[String],
) -> Result<String, String> {
    if !icon_url.starts_with("https://") {
        return Err("图标地址必须为 HTTPS".to_owned());
    }
    if !is_hex(icon_sha256, 64) {
        return Err("图标 SHA-256 无效".to_owned());
    }

    let hash_prefix = &icon_sha256[..16];
    let icon_dir = cache_dir.join("icons");
    let target = icon_dir.join(format!("{app_id}-{hash_prefix}.png"));

    if !target.is_file() {
        let client = crate::source_engine::restricted_client(hosts, Duration::from_secs(20))?;
        let response = client
            .get(icon_url)
            .send()
            .await
            .map_err(|error| format!("下载图标失败：{error}"))?
            .error_for_status()
            .map_err(|error| format!("图标服务器返回错误：{error}"))?;
        if response.url().scheme() != "https"
            || !crate::source_engine::host_allowed(response.url().host_str(), hosts)
        {
            return Err("图标来源于未授权域名".to_owned());
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("读取图标内容失败：{error}"))?;
        if bytes.len() > 1024 * 1024 {
            return Err("图标大小异常".to_owned());
        }
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(icon_sha256) {
            return Err("图标 SHA-256 校验失败".to_owned());
        }
        tokio::fs::create_dir_all(&icon_dir)
            .await
            .map_err(|error| format!("无法创建图标缓存目录：{error}"))?;
        tokio::fs::write(&target, &bytes)
            .await
            .map_err(|error| format!("写入图标缓存失败：{error}"))?;
    }

    let bytes = tokio::fs::read(&target)
        .await
        .map_err(|error| format!("读取图标缓存失败：{error}"))?;
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

fn is_hex(input: &str, len: usize) -> bool {
    input.len() == len && input.bytes().all(|byte| byte.is_ascii_hexdigit())
}
