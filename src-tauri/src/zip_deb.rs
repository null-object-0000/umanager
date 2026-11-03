//! ZIPs are untrusted containers, never directories to extract wholesale.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

pub const MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipDebArtifact {
    pub member: String,
    pub size: u64,
    pub sha256: String,
}

pub fn valid_member(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', '\0'])
        && name.ends_with(".deb")
        && name != ".deb"
        && !name.starts_with('.')
}

pub fn valid_digest(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

pub fn extract_deb(
    archive_path: &Path,
    destination: &Path,
    member: &str,
    expected_size: u64,
    expected_sha256: &str,
    cancel: &crate::source_engine::CancelCheck,
) -> Result<(), String> {
    if !valid_member(member)
        || expected_size == 0
        || expected_size > MAX_ARTIFACT_BYTES
        || !valid_digest(expected_sha256)
    {
        return Err("ZIP 内安装包的签名元数据无效".into());
    }
    let input = File::open(archive_path).map_err(|e| format!("无法读取 ZIP：{e}"))?;
    let mut zip = zip::ZipArchive::new(input).map_err(|e| format!("ZIP 格式无效：{e}"))?;
    if zip.len() > 1024 {
        return Err("ZIP 文件条目过多".into());
    }
    let mut selected = None;
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|e| format!("ZIP 条目无效：{e}"))?;
        if entry.name().ends_with(".deb") {
            if selected.is_some()
                || entry.name() != member
                || !valid_member(entry.name())
                || entry.size() != expected_size
                || entry.is_dir()
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
                || !matches!(
                    entry.compression(),
                    zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
                )
            {
                return Err("ZIP 必须包含唯一、大小一致且名称匹配的普通 DEB 文件".into());
            }
            selected = Some(index);
        }
    }
    let index = selected.ok_or_else(|| "ZIP 内没有签名元数据指定的 DEB".to_owned())?;
    let mut entry = zip
        .by_index(index)
        .map_err(|e| format!("无法读取 DEB 条目：{e}"))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| format!("无法创建解压临时文件：{e}"))?;
    let result = (|| {
        let mut buffer = [0_u8; 64 * 1024];
        let mut size = 0_u64;
        let mut hash = Sha256::new();
        loop {
            if cancel() {
                return Err(crate::source_engine::CANCELED_ERROR.to_owned());
            }
            let count = entry
                .read(&mut buffer)
                .map_err(|e| format!("解压 DEB 失败：{e}"))?;
            if count == 0 {
                break;
            }
            size = size.checked_add(count as u64).ok_or("解压大小溢出")?;
            if size > expected_size {
                return Err("解压内容超过签名声明大小".into());
            }
            hash.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .map_err(|e| format!("写入 DEB 失败：{e}"))?;
        }
        if size != expected_size
            || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected_sha256)
        {
            return Err("ZIP 内 DEB 的大小或 SHA-256 与签名元数据不一致".into());
        }
        output.sync_all().map_err(|e| format!("同步 DEB 失败：{e}"))
    })();
    drop(output);
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Arc;
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn fixture(names: &[&str]) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "umanager-zip-test-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(b"test deb payload").unwrap();
        }
        std::fs::write(
            dir.join("source.zip"),
            writer.finish().unwrap().into_inner(),
        )
        .unwrap();
        (dir, format!("{:x}", Sha256::digest(b"test deb payload")))
    }

    #[test]
    fn extracts_only_the_signed_member_and_checks_hash() {
        let (dir, hash) = fixture(&["app.deb", "readme.txt"]);
        extract_deb(
            &dir.join("source.zip"),
            &dir.join("output.deb"),
            "app.deb",
            16,
            &hash,
            &(Arc::new(|| false) as crate::source_engine::CancelCheck),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.join("output.deb")).unwrap(),
            b"test deb payload"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_traversal_multiple_members_wrong_size_hash_and_cancellation() {
        for names in [&["../app.deb"][..], &["app.deb", "other.deb"][..]] {
            let (dir, hash) = fixture(names);
            assert!(
                extract_deb(
                    &dir.join("source.zip"),
                    &dir.join("output.deb"),
                    "app.deb",
                    16,
                    &hash,
                    &(Arc::new(|| false) as crate::source_engine::CancelCheck)
                )
                .is_err()
            );
            assert!(!dir.join("output.deb").exists());
            std::fs::remove_dir_all(dir).unwrap();
        }
        let (dir, hash) = fixture(&["app.deb"]);
        for (size, digest, canceled) in [
            (15, hash.as_str(), false),
            (
                16,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                false,
            ),
            (16, hash.as_str(), true),
        ] {
            let cancel: crate::source_engine::CancelCheck = Arc::new(move || canceled);
            assert!(
                extract_deb(
                    &dir.join("source.zip"),
                    &dir.join("output.deb"),
                    "app.deb",
                    size,
                    digest,
                    &cancel
                )
                .is_err()
            );
            assert!(!dir.join("output.deb").exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_symlink_members() {
        let (dir, hash) = fixture(&["app.deb"]);
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink(
                "app.deb",
                "/etc/passwd",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        std::fs::write(
            dir.join("source.zip"),
            writer.finish().unwrap().into_inner(),
        )
        .unwrap();
        let cancel: crate::source_engine::CancelCheck = Arc::new(|| false);
        assert!(
            extract_deb(
                &dir.join("source.zip"),
                &dir.join("output.deb"),
                "app.deb",
                16,
                &hash,
                &cancel
            )
            .is_err()
        );
        assert!(!dir.join("output.deb").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires UMANAGER_APIFOX_ZIP pointing to the official 2.8.53 ZIP fixture"]
    fn official_apifox_zip_produces_the_verified_deb() {
        let input = std::path::PathBuf::from(
            std::env::var("UMANAGER_APIFOX_ZIP").expect("ZIP fixture path"),
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&input).unwrap())),
            "79e74ef128a0193b5d7ad6411175a485fcc8a75fbb2869e290af04f23edf535c"
        );
        let (dir, _) = fixture(&["unused.deb"]);
        let output = dir.join("apifox.deb");
        let cancel: crate::source_engine::CancelCheck = Arc::new(|| false);
        extract_deb(
            &input,
            &output,
            "apifox_2.8.53_amd64.deb",
            156009548,
            "5c3f19a0c9298ac575871e026160abec4d5aaf1bae9256645913a9e25e0656e3",
            &cancel,
        )
        .unwrap();
        let metadata = std::process::Command::new("/usr/bin/dpkg-deb")
            .args(["-f"])
            .arg(&output)
            .args(["Package", "Version", "Architecture"])
            .output()
            .unwrap();
        assert!(metadata.status.success());
        assert_eq!(
            String::from_utf8(metadata.stdout).unwrap(),
            "Package: apifox\nVersion: 2.8.53\nArchitecture: amd64\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
