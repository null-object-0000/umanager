use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub(super) fn home() -> Result<PathBuf, String> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("无法确定用户目录")?);
    let uid = fs::metadata("/proc/self").map_err(err)?.uid();
    let meta = fs::symlink_metadata(&home).map_err(err)?;
    if uid == 0 || !home.is_absolute() || meta.uid() != uid || meta.file_type().is_symlink() {
        return Err("归档应用必须以普通用户运行，用户目录须由当前用户拥有且不能是符号链接".into());
    }
    Ok(home)
}
pub(super) fn check_path(home: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(home).map_err(|_| "路径不在用户目录内")?;
    let uid = fs::metadata(home).map_err(err)?.uid();
    let mut cursor = home.to_path_buf();
    for c in relative.components() {
        if !matches!(c, Component::Normal(_)) {
            return Err("非法路径分量".into());
        }
        cursor.push(c);
        match fs::symlink_metadata(&cursor) {
            Ok(m) if m.file_type().is_symlink() || m.uid() != uid => {
                return Err(format!(
                    "路径必须由当前用户拥有且不能是符号链接：{}",
                    cursor.display()
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
    }
    Ok(())
}
pub(super) fn atomic_write(home: &Path, path: &Path, bytes: &[u8]) -> Result<(), String> {
    check_path(home, path)?;
    let temp = path.with_extension("umanager-new");
    check_path(home, &temp)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(err)?;
    let result = (|| {
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        fs::rename(&temp, path).map_err(err)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub(super) fn desktop_quote(path: &Path) -> Result<String, String> {
    let text = path.to_str().ok_or("启动路径不是 UTF-8")?;
    if text.contains(['\n', '\r', '\0']) {
        return Err("启动路径包含控制字符".into());
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\\\\\")
            .replace('"', "\\\\\"")
            .replace('`', "\\\\`")
            .replace('$', "\\\\$")
            .replace('%', "%%")
    ))
}
