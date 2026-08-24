use super::{ARCHITECTURE, PACKAGE_NAME, REPOSITORY_HOST};
use crate::scanner;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const APT_CACHE_BIN: &str = "/usr/bin/apt-cache";
const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPlan {
    pub(crate) package_name: String,
    pub(crate) version: String,
    pub(crate) architecture: String,
    repository_url: String,
    download_url: String,
    file_name: String,
    pub(crate) expected_size: u64,
    pub(crate) expected_sha256: String,
    pub(crate) target_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResult {
    pub(crate) plan: DownloadPlan,
    pub(crate) actual_size: u64,
    pub(crate) actual_sha256: String,
    package_name: String,
    version: String,
    architecture: String,
    reused_existing_file: bool,
    verified: bool,
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

pub fn build_plan(cache_dir: &Path) -> Result<DownloadPlan, String> {
    let scan = scanner::scan()?;
    let package = scan
        .packages
        .iter()
        .find(|item| item.package_name == PACKAGE_NAME)
        .ok_or_else(|| "未检测到已安装的 Visual Studio Code（软件包 code）".to_owned())?;
    let version = package
        .candidate_version
        .as_deref()
        .ok_or_else(|| "APT 缓存中没有 VS Code 候选版本，无法生成下载计划".to_owned())?;
    if package.architecture != ARCHITECTURE {
        return Err(format!(
            "VS Code 架构为 {}，当前适配器只允许 {ARCHITECTURE}",
            package.architecture
        ));
    }
    let repository_url = package
        .source_url
        .as_deref()
        .filter(|url| has_allowed_https_host(url))
        .ok_or_else(|| "未发现可信的 Microsoft VS Code APT 仓库".to_owned())?;

    let output = scanner::locale_stable_command(APT_CACHE_BIN)
        .arg("show")
        .arg(format!("{PACKAGE_NAME}={version}"))
        .output()
        .map_err(|error| format!("无法读取 VS Code APT 包索引：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "读取 VS Code APT 包索引失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let record = parse_package_index(&String::from_utf8_lossy(&output.stdout), version)?;
    validate_index_record(&record, version)?;
    let file_name = safe_deb_file_name(&record.filename)?;
    let download_url = join_repository_url(repository_url, &record.filename)?;
    let target_path = cache_dir.join("downloads").join(&file_name);

    Ok(DownloadPlan {
        package_name: record.package,
        version: record.version,
        architecture: record.architecture,
        repository_url: repository_url.to_owned(),
        download_url,
        file_name,
        expected_size: record.size,
        expected_sha256: record.sha256,
        target_path: target_path.to_string_lossy().into_owned(),
    })
}

pub async fn download_and_verify(cache_dir: PathBuf) -> Result<DownloadResult, String> {
    let plan = tokio::task::spawn_blocking(move || build_plan(&cache_dir))
        .await
        .map_err(|error| format!("VS Code 下载计划任务失败：{error}"))??;
    let target_path = PathBuf::from(&plan.target_path);
    let parent = target_path
        .parent()
        .ok_or_else(|| "无效的 VS Code 下载缓存路径".to_owned())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("无法创建下载缓存目录：{error}"))?;

    if target_path.exists() {
        let verified = verify_file(&target_path, &plan).await?;
        return Ok(result_from_verified(plan, verified, true));
    }

    let temporary_path = temporary_path(parent, &plan.file_name);
    let outcome = download_to_temporary_file(&plan, &temporary_path).await;
    let verified = match outcome {
        Ok(verified) => verified,
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(error);
        }
    };

    if let Err(error) = tokio::fs::hard_link(&temporary_path, &target_path).await {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(format!(
            "无法将已校验文件安全写入缓存（目标文件可能已存在）：{error}"
        ));
    }
    tokio::fs::remove_file(&temporary_path)
        .await
        .map_err(|error| format!("已写入缓存，但无法清理临时下载文件：{error}"))?;
    Ok(result_from_verified(plan, verified, false))
}

pub async fn verify_cached(cache_dir: PathBuf) -> Result<DownloadResult, String> {
    let plan = tokio::task::spawn_blocking(move || build_plan(&cache_dir))
        .await
        .map_err(|error| format!("VS Code 缓存校验计划任务失败：{error}"))??;
    let target_path = PathBuf::from(&plan.target_path);
    if !target_path.is_file() {
        return Err("VS Code 安装包尚未下载或不再位于缓存中".to_owned());
    }
    let verified = verify_file(&target_path, &plan).await?;
    Ok(result_from_verified(plan, verified, true))
}

async fn download_to_temporary_file(
    plan: &DownloadPlan,
    temporary_path: &Path,
) -> Result<VerifiedFile, String> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("VS Code 下载重定向次数超过限制");
            }
            if attempt.url().scheme() == "https"
                && attempt.url().host_str() == Some(REPOSITORY_HOST)
            {
                attempt.follow()
            } else {
                attempt.error("VS Code 下载重定向到未授权域名")
            }
        }))
        .user_agent("UManager/0.1")
        .build()
        .map_err(|error| format!("无法创建安全下载客户端：{error}"))?;
    let mut response = client
        .get(&plan.download_url)
        .send()
        .await
        .map_err(|error| format!("下载 VS Code 失败：{error}"))?
        .error_for_status()
        .map_err(|error| format!("VS Code 下载服务器返回错误：{error}"))?;
    if response.url().scheme() != "https" || response.url().host_str() != Some(REPOSITORY_HOST) {
        return Err("VS Code 下载最终地址不属于允许的 Microsoft 域名".to_owned());
    }
    if let Some(content_length) = response.content_length()
        && content_length != plan.expected_size
    {
        return Err(format!(
            "VS Code 下载大小与官方索引不一致：预期 {}，响应为 {content_length}",
            plan.expected_size
        ));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .await
        .map_err(|error| format!("无法创建临时下载文件：{error}"))?;
    let mut hasher = Sha256::new();
    let mut actual_size = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取 VS Code 下载内容失败：{error}"))?
    {
        actual_size = actual_size
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| "VS Code 下载大小溢出".to_owned())?;
        if actual_size > plan.expected_size {
            return Err("VS Code 下载内容超过官方索引声明的大小".to_owned());
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("写入 VS Code 临时文件失败：{error}"))?;
    }
    file.flush()
        .await
        .map_err(|error| format!("刷新 VS Code 临时文件失败：{error}"))?;
    file.sync_all()
        .await
        .map_err(|error| format!("同步 VS Code 临时文件失败：{error}"))?;
    drop(file);

    let actual_sha256 = format!("{:x}", hasher.finalize());
    validate_size_and_hash(plan, actual_size, &actual_sha256)?;
    let metadata = inspect_deb(temporary_path).await?;
    validate_deb_metadata(plan, &metadata)?;
    Ok(VerifiedFile {
        actual_size,
        actual_sha256,
        metadata,
    })
}

async fn verify_file(path: &Path, plan: &DownloadPlan) -> Result<VerifiedFile, String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("无法读取已有 VS Code 缓存文件：{error}"))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    let mut actual_size = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取已有 VS Code 缓存文件失败：{error}"))?;
        if count == 0 {
            break;
        }
        actual_size += count as u64;
        hasher.update(&buffer[..count]);
    }
    let actual_sha256 = format!("{:x}", hasher.finalize());
    validate_size_and_hash(plan, actual_size, &actual_sha256)?;
    let metadata = inspect_deb(path).await?;
    validate_deb_metadata(plan, &metadata)?;
    Ok(VerifiedFile {
        actual_size,
        actual_sha256,
        metadata,
    })
}

struct VerifiedFile {
    actual_size: u64,
    actual_sha256: String,
    metadata: DebMetadata,
}

fn result_from_verified(
    plan: DownloadPlan,
    verified: VerifiedFile,
    reused_existing_file: bool,
) -> DownloadResult {
    DownloadResult {
        plan,
        actual_size: verified.actual_size,
        actual_sha256: verified.actual_sha256,
        package_name: verified.metadata.package,
        version: verified.metadata.version,
        architecture: verified.metadata.architecture,
        reused_existing_file,
        verified: true,
    }
}

async fn inspect_deb(path: &Path) -> Result<DebMetadata, String> {
    let owned_path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        Ok(DebMetadata {
            package: read_deb_field(&owned_path, "Package")?,
            version: read_deb_field(&owned_path, "Version")?,
            architecture: read_deb_field(&owned_path, "Architecture")?,
        })
    })
    .await
    .map_err(|error| format!("VS Code 包元数据检查任务失败：{error}"))?
}

fn read_deb_field(path: &Path, field: &str) -> Result<String, String> {
    let output = scanner::locale_stable_command(DPKG_DEB_BIN)
        .arg("--field")
        .arg(path)
        .arg(field)
        .output()
        .map_err(|error| format!("无法读取 .deb 字段 {field}：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "读取 .deb 字段 {field} 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn parse_package_index(input: &str, expected_version: &str) -> Result<PackageIndexRecord, String> {
    for paragraph in input.split("\n\n") {
        let fields: HashMap<_, _> = paragraph
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim(), value.trim()))
            .collect();
        if fields.get("Package") == Some(&PACKAGE_NAME)
            && fields.get("Version") == Some(&expected_version)
            && fields.get("Architecture") == Some(&ARCHITECTURE)
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
        "APT 包索引中没有找到 {PACKAGE_NAME} {expected_version} {ARCHITECTURE}"
    ))
}

fn required_field<'a>(fields: &'a HashMap<&str, &str>, name: &str) -> Result<&'a str, String> {
    fields
        .get(name)
        .copied()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("APT 包索引缺少字段 {name}"))
}

fn validate_index_record(record: &PackageIndexRecord, version: &str) -> Result<(), String> {
    if record.package != PACKAGE_NAME
        || record.architecture != ARCHITECTURE
        || record.version != version
    {
        return Err("APT 包索引与 VS Code 下载计划不匹配".to_owned());
    }
    if record.size == 0 {
        return Err("APT 包索引声明的文件大小为零".to_owned());
    }
    if record.sha256.len() != 64 || !record.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("APT 包索引中的 SHA256 无效".to_owned());
    }
    Ok(())
}

fn validate_size_and_hash(
    plan: &DownloadPlan,
    actual_size: u64,
    actual_sha256: &str,
) -> Result<(), String> {
    if actual_size != plan.expected_size {
        return Err(format!(
            "VS Code 文件大小校验失败：预期 {}，实际 {actual_size}",
            plan.expected_size
        ));
    }
    if !actual_sha256.eq_ignore_ascii_case(&plan.expected_sha256) {
        return Err("VS Code SHA-256 校验失败，文件不会进入缓存".to_owned());
    }
    Ok(())
}

fn validate_deb_metadata(plan: &DownloadPlan, metadata: &DebMetadata) -> Result<(), String> {
    if metadata.package != plan.package_name
        || metadata.version != plan.version
        || metadata.architecture != plan.architecture
    {
        return Err(format!(
            "VS Code .deb 元数据不匹配：得到 {} {} {}",
            metadata.package, metadata.version, metadata.architecture
        ));
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

fn join_repository_url(repository: &str, filename: &str) -> Result<String, String> {
    if !has_allowed_https_host(repository) {
        return Err("VS Code 仓库 URL 不在允许列表中".to_owned());
    }
    safe_deb_file_name(filename)?;
    Ok(format!(
        "{}/{}",
        repository.trim_end_matches('/'),
        filename.trim_start_matches('/')
    ))
}

fn has_allowed_https_host(url: &str) -> bool {
    url.strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host.eq_ignore_ascii_case(REPOSITORY_HOST))
}

fn temporary_path(parent: &Path, file_name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        ".{file_name}.{}.{}.part",
        std::process::id(),
        nonce
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE_INDEX: &str = r#"Package: code
Version: 1.134.0-1787078834
Architecture: amd64
Filename: pool/main/c/code/code_1.134.0-1787078834_amd64.deb
Size: 105000000
SHA256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
Description: Visual Studio Code
"#;

    #[test]
    fn parses_the_exact_apt_index_record() {
        let record = parse_package_index(PACKAGE_INDEX, "1.134.0-1787078834").unwrap();
        assert_eq!(record.package, "code");
        assert_eq!(record.architecture, "amd64");
        assert_eq!(record.size, 105_000_000);
    }

    #[test]
    fn builds_only_an_allowed_repository_url() {
        let url = join_repository_url(
            "https://packages.microsoft.com/repos/code",
            "pool/main/c/code/code_1.0_amd64.deb",
        )
        .unwrap();
        assert_eq!(
            url,
            "https://packages.microsoft.com/repos/code/pool/main/c/code/code_1.0_amd64.deb"
        );
        assert!(
            join_repository_url(
                "https://packages.microsoft.com.evil.example/repos/code",
                "pool/code.deb"
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_path_traversal_from_the_package_index() {
        assert!(safe_deb_file_name("../../code.deb").is_err());
        assert!(safe_deb_file_name("/tmp/code.deb").is_err());
        assert_eq!(
            safe_deb_file_name("pool/main/c/code/code_1.0_amd64.deb").unwrap(),
            "code_1.0_amd64.deb"
        );
    }

    #[test]
    fn rejects_invalid_hashes_and_sizes() {
        let mut record = parse_package_index(PACKAGE_INDEX, "1.134.0-1787078834").unwrap();
        record.sha256 = "not-a-hash".to_owned();
        assert!(validate_index_record(&record, "1.134.0-1787078834").is_err());
        record.sha256 = "0".repeat(64);
        record.size = 0;
        assert!(validate_index_record(&record, "1.134.0-1787078834").is_err());
    }
}
