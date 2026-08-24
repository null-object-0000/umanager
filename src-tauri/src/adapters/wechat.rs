use crate::scanner::{self, UpdateState};
use reqwest::header::{CONTENT_RANGE, RANGE};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::io::AsyncWriteExt;

const PACKAGE_NAME: &str = "wechat";
const ARCHITECTURE: &str = "amd64";
const OFFICIAL_PAGE: &str = "https://linux.weixin.qq.com/";
const DOWNLOAD_URL: &str = "https://dldir1v6.qq.com/weixin/Universal/Linux/WeChatLinux_x86_64.deb";
const PAGE_HOST: &str = "linux.weixin.qq.com";
const DOWNLOAD_HOST: &str = "dldir1v6.qq.com";
const CONTROL_PREFIX_BYTES: u64 = 8 * 1024 * 1024;
const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatDetails {
    application_id: &'static str,
    display_name: &'static str,
    package_name: &'static str,
    installed_version: String,
    website_version: String,
    package_version: String,
    architecture: String,
    update_state: UpdateState,
    official_page: &'static str,
    download_url: &'static str,
    expected_size: u64,
    source_trusted: bool,
    evidence: Vec<WechatEvidence>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WechatEvidence {
    label: &'static str,
    actual: String,
    expected: &'static str,
    passed: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct PageMetadata {
    version: String,
    download_url: String,
}

#[derive(Debug)]
struct PackageMetadata {
    package_name: String,
    version: String,
    architecture: String,
    total_size: u64,
}

pub async fn load_details(cache_dir: PathBuf) -> Result<WechatDetails, String> {
    let scan = tauri::async_runtime::spawn_blocking(scanner::scan)
        .await
        .map_err(|error| format!("微信本机扫描任务失败：{error}"))??;
    let installed = scan
        .packages
        .into_iter()
        .find(|package| package.package_name == PACKAGE_NAME)
        .ok_or_else(|| "未检测到已安装的微信 Linux 版".to_owned())?;
    let page = fetch_page_metadata().await?;
    let package = fetch_package_metadata_prefix(&cache_dir).await?;
    if page.download_url != DOWNLOAD_URL {
        return Err("微信官网的 x86_64 .deb 下载地址发生变化，需要更新适配策略".to_owned());
    }
    if package.package_name != PACKAGE_NAME || package.architecture != ARCHITECTURE {
        return Err("微信官方下载包的包名或架构不符合适配策略".to_owned());
    }
    if !version_has_display_prefix(&package.version, &page.version) {
        return Err("微信网页版本与官方下载包完整版本不一致".to_owned());
    }
    let update_state = if debian_version_is_newer(&installed.installed_version, &package.version) {
        UpdateState::UpdateAvailable
    } else {
        UpdateState::UpToDate
    };
    Ok(WechatDetails {
        application_id: "wechat",
        display_name: "微信",
        package_name: PACKAGE_NAME,
        installed_version: installed.installed_version,
        website_version: page.version,
        package_version: package.version,
        architecture: package.architecture,
        update_state,
        official_page: OFFICIAL_PAGE,
        download_url: DOWNLOAD_URL,
        expected_size: package.total_size,
        source_trusted: true,
        evidence: vec![
            WechatEvidence {
                label: "官网域名",
                actual: PAGE_HOST.to_owned(),
                expected: PAGE_HOST,
                passed: true,
            },
            WechatEvidence {
                label: "下载域名",
                actual: DOWNLOAD_HOST.to_owned(),
                expected: DOWNLOAD_HOST,
                passed: true,
            },
            WechatEvidence {
                label: "Debian 软件包名",
                actual: PACKAGE_NAME.to_owned(),
                expected: PACKAGE_NAME,
                passed: true,
            },
            WechatEvidence {
                label: "软件包架构",
                actual: ARCHITECTURE.to_owned(),
                expected: ARCHITECTURE,
                passed: true,
            },
        ],
    })
}

async fn fetch_page_metadata() -> Result<PageMetadata, String> {
    let client = restricted_client(PAGE_HOST)?;
    let response = client
        .get(OFFICIAL_PAGE)
        .send()
        .await
        .map_err(|error| format!("读取微信 Linux 官网失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("微信 Linux 官网返回错误：{error}"))?;
    if response.url().scheme() != "https" || response.url().host_str() != Some(PAGE_HOST) {
        return Err("微信官网重定向到未授权域名".to_owned());
    }
    if response
        .content_length()
        .is_some_and(|size| size > 1024 * 1024)
    {
        return Err("微信官网页面大小异常".to_owned());
    }
    let html = response
        .text()
        .await
        .map_err(|error| format!("读取微信官网页面内容失败：{error}"))?;
    if html.len() > 1024 * 1024 {
        return Err("微信官网页面大小异常".to_owned());
    }
    parse_page_metadata(&html)
}

async fn fetch_package_metadata_prefix(cache_dir: &Path) -> Result<PackageMetadata, String> {
    let client = restricted_client(DOWNLOAD_HOST)?;
    let response = client
        .get(DOWNLOAD_URL)
        .header(RANGE, format!("bytes=0-{}", CONTROL_PREFIX_BYTES - 1))
        .send()
        .await
        .map_err(|error| format!("读取微信安装包控制信息失败：{error}"))?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err("微信下载服务器未按预期返回 Range 内容，拒绝下载完整安装包".to_owned());
    }
    if response.url().scheme() != "https" || response.url().host_str() != Some(DOWNLOAD_HOST) {
        return Err("微信安装包重定向到未授权域名".to_owned());
    }
    let total_size = parse_content_range_total(
        response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| "微信安装包 Range 响应缺少 Content-Range".to_owned())?,
    )?;
    let metadata_dir = cache_dir.join("metadata");
    tokio::fs::create_dir_all(&metadata_dir)
        .await
        .map_err(|error| format!("无法创建微信元数据缓存目录：{error}"))?;
    let path = metadata_dir.join(format!("wechat-control-{}.deb", std::process::id()));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .await
        .map_err(|error| format!("无法创建微信控制信息临时文件：{error}"))?;
    let mut response = response;
    let mut written = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取微信安装包控制信息失败：{error}"))?
    {
        written += chunk.len() as u64;
        if written > CONTROL_PREFIX_BYTES {
            let _ = tokio::fs::remove_file(&path).await;
            return Err("微信安装包 Range 响应超过允许大小".to_owned());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("写入微信控制信息失败：{error}"))?;
    }
    file.flush()
        .await
        .map_err(|error| format!("刷新微信控制信息失败：{error}"))?;
    drop(file);
    let inspect_path = path.clone();
    let inspection = tauri::async_runtime::spawn_blocking(move || {
        inspect_partial_deb(&inspect_path, total_size)
    })
    .await
    .map_err(|error| format!("微信安装包元数据任务失败：{error}"))?;
    let cleanup = tokio::fs::remove_file(&path)
        .await
        .map_err(|error| format!("无法清理微信控制信息临时文件：{error}"));
    let metadata = inspection?;
    cleanup?;
    Ok(metadata)
}

fn parse_page_metadata(html: &str) -> Result<PageMetadata, String> {
    let marker = "main-section__bd-version\"";
    let after_marker = html
        .find(marker)
        .and_then(|index| {
            html[index + marker.len()..]
                .find('>')
                .map(|offset| index + marker.len() + offset + 1)
        })
        .ok_or_else(|| "微信官网页面中未找到版本节点".to_owned())?;
    let version = html[after_marker..]
        .split('<')
        .next()
        .map(str::trim)
        .filter(|value| valid_display_version(value))
        .ok_or_else(|| "微信官网展示版本格式无效".to_owned())?
        .to_owned();
    let download_url = extract_href_for_filename(html, "WeChatLinux_x86_64.deb")?;
    Ok(PageMetadata {
        version,
        download_url,
    })
}

fn extract_href_for_filename(html: &str, filename: &str) -> Result<String, String> {
    let filename_index = html
        .find(filename)
        .ok_or_else(|| "微信官网未提供 x86_64 Debian 安装包".to_owned())?;
    let before = &html[..filename_index];
    let href_index = before
        .rfind("href=\"")
        .ok_or_else(|| "微信官网下载链接格式无效".to_owned())?
        + 6;
    let value = html[href_index..]
        .split('"')
        .next()
        .ok_or_else(|| "微信官网下载链接格式无效".to_owned())?;
    Ok(value.to_owned())
}

fn inspect_partial_deb(path: &Path, total_size: u64) -> Result<PackageMetadata, String> {
    Ok(PackageMetadata {
        package_name: deb_field(path, "Package")?,
        version: deb_field(path, "Version")?,
        architecture: deb_field(path, "Architecture")?,
        total_size,
    })
}

fn deb_field(path: &Path, field: &str) -> Result<String, String> {
    let output = clean_command(DPKG_DEB_BIN)
        .arg("--field")
        .arg(path)
        .arg(field)
        .output()
        .map_err(|error| format!("无法读取微信 .deb {field} 字段：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "微信 .deb 控制信息无效：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn parse_content_range_total(value: &str) -> Result<u64, String> {
    value
        .split_once('/')
        .and_then(|(_, total)| total.parse().ok())
        .filter(|total| *total > CONTROL_PREFIX_BYTES)
        .ok_or_else(|| "微信安装包 Content-Range 总大小无效".to_owned())
}

fn valid_display_version(value: &str) -> bool {
    let components: Vec<_> = value.split('.').collect();
    components.len() == 3
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

fn debian_version_is_newer(installed: &str, candidate: &str) -> bool {
    clean_command("/usr/bin/dpkg")
        .args(["--compare-versions", installed, "lt", candidate])
        .status()
        .is_ok_and(|status| status.success())
}

fn restricted_client(allowed_host: &'static str) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("微信请求重定向次数超过限制");
            }
            if attempt.url().scheme() == "https" && attempt.url().host_str() == Some(allowed_host) {
                attempt.follow()
            } else {
                attempt.error("微信请求重定向到未授权域名")
            }
        }))
        .user_agent("UManager/0.1")
        .build()
        .map_err(|error| format!("无法创建微信安全请求客户端：{error}"))
}

fn clean_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_server_rendered_wechat_page() {
        let html = r#"<div class="main-section__bd-version" data-v-x>4.1.1</div><a href="https://dldir1v6.qq.com/weixin/Universal/Linux/WeChatLinux_x86_64.deb">deb</a>"#;
        assert_eq!(
            parse_page_metadata(html).unwrap(),
            PageMetadata {
                version: "4.1.1".to_owned(),
                download_url: DOWNLOAD_URL.to_owned(),
            }
        );
    }

    #[test]
    fn matches_only_a_complete_version_prefix() {
        assert!(version_has_display_prefix("4.1.1.8", "4.1.1"));
        assert!(version_has_display_prefix("4.1.1", "4.1.1"));
        assert!(!version_has_display_prefix("4.1.10.1", "4.1.1"));
    }

    #[test]
    fn parses_content_range_total() {
        assert_eq!(
            parse_content_range_total("bytes 0-8388607/212419528").unwrap(),
            212_419_528
        );
        assert!(parse_content_range_total("bytes 0-100/*").is_err());
    }
}
