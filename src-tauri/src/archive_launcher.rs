//! Copy the ordinary-user launch adapter independently of the UManager DEB.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
pub(crate) use umanager_launcher::{prepare_desktop, remove_icon_alias, sync_icon_alias};

fn install_launcher(home: &Path) -> Result<(), String> {
    let source = [
        PathBuf::from("/usr/libexec/umanager-antigravity-launcher"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/umanager-launcher/target/release/umanager-antigravity-launcher"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or("缺少 Antigravity 启动适配器，请重新安装 UManager DEB")?;
    let metadata = fs::metadata(&source).map_err(|e| e.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("启动适配器大小异常".into());
    }
    let bytes = fs::read(source).map_err(|e| e.to_string())?;
    install_bytes(home, &bytes)
}

fn install_bytes(home: &Path, bytes: &[u8]) -> Result<(), String> {
    let destination = home.join(".local/share/umanager/bin/umanager-antigravity-launcher");
    crate::user_archive::check_path(home, &destination)?;
    if !bytes.starts_with(b"\x7fELF") {
        return Err("启动适配器不是 ELF 程序".into());
    }
    if fs::read(&destination).is_ok_and(|old| old == bytes) {
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    fs::create_dir_all(destination.parent().ok_or("启动目录无效")?).map_err(|e| e.to_string())?;
    let temporary = destination.with_extension("umanager-new");
    crate::user_archive::check_path(home, &temporary)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, &destination).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(crate) fn ensure_installed(home: &Path, id: &str) -> Result<(), String> {
    if matches!(id, "antigravity" | "antigravity-ide") {
        install_launcher(home)?;
    }
    Ok(())
}
pub fn refresh_installed() -> Result<(), String> {
    let home = crate::user_archive::home()?;
    if ["antigravity", "antigravity-ide"].iter().any(|id| {
        home.join(".local/share/umanager/apps")
            .join(id)
            .join("installed.json")
            .is_file()
    }) {
        install_launcher(&home)?;
        umanager_launcher::refresh_installed()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    #[test]
    fn adapter_copy_is_executable_idempotent_and_rejects_links() {
        let home =
            std::env::temp_dir().join(format!("umanager-adapter-copy-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let bytes = b"\x7fELFfixture";
        install_bytes(&home, bytes).unwrap();
        let path = home.join(".local/share/umanager/bin/umanager-antigravity-launcher");
        let inode = fs::metadata(&path).unwrap().ino();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        install_bytes(&home, bytes).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, 0o755);
        assert!(install_bytes(&home, b"not ELF").is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/bin/true", &path).unwrap();
        assert!(install_bytes(&home, bytes).is_err());
        fs::remove_dir_all(home).unwrap();
    }
}
