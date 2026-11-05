//! Unprivileged desktop tar.gz installs. No helper, shell, or archive scripts.
//! Download facts come exclusively from the verified feed; extraction is bounded
//! and never follows archive-created links. Install plans have their own format
//! and cannot be submitted to the privileged DEB helper.
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;
use umanager_catalog::{Application, SourceSpec};

const MAX_UNPACKED: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 100
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn home() -> Result<PathBuf, String> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("无法确定用户目录")?);
    let uid = fs::metadata("/proc/self").map_err(err)?.uid();
    let meta = fs::symlink_metadata(&home).map_err(err)?;
    if uid == 0 || !home.is_absolute() || meta.uid() != uid || meta.file_type().is_symlink() {
        return Err("归档应用必须以普通用户运行，用户目录须由当前用户拥有且不能是符号链接".into());
    }
    Ok(home)
}
// Reject symlinks in all managed path ancestors, including XDG directories.
fn check_path(home: &Path, path: &Path) -> Result<(), String> {
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
fn root(home: &Path, app: &Application) -> Result<PathBuf, String> {
    if !app.is_user_archive() || !safe_id(&app.application_id) || !safe_id(&app.package_name) {
        return Err("不是有效的用户归档应用".into());
    }
    let root = home
        .join(".local/share/umanager/apps")
        .join(&app.application_id);
    check_path(home, &root)?;
    Ok(root)
}
fn desktop(home: &Path, app: &Application) -> PathBuf {
    home.join(".local/share/applications")
        .join(format!("umanager-{}.desktop", app.application_id))
}
fn spec(app: &Application) -> Result<(&str, &str), String> {
    let SourceSpec::TarGz {
        executable,
        icon_path,
        ..
    } = &app.source
    else {
        return Err("不是 tar.gz 软件源".into());
    };
    if !safe_relative(Path::new(executable)) || !safe_relative(Path::new(icon_path)) {
        return Err("归档程序或图标路径无效".into());
    }
    Ok((executable, icon_path))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Installed {
    version: String,
    sha256: String,
}
fn state(home: &Path, app: &Application) -> Result<Option<Installed>, String> {
    let root = root(home, app)?;
    let path = root.join("installed.json");
    check_path(home, &path)?;
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err(e)),
    };
    let s: Installed = serde_json::from_slice(&bytes).map_err(err)?;
    if !crate::zip_deb::valid_digest(&s.sha256) || s.version.is_empty() {
        return Err("归档安装记录无效".into());
    }
    let executable = root.join(&s.sha256).join(spec(app)?.0);
    check_path(home, &executable)?;
    if !executable.is_file() {
        return Err("归档安装文件缺失，请重新安装".into());
    }
    Ok(Some(s))
}
pub fn installed_version(app: &Application) -> Result<Option<String>, String> {
    state(&home()?, app).map(|s| s.map(|s| s.version))
}
pub fn desktop_path(app: &Application) -> Result<PathBuf, String> {
    let home = home()?;
    state(&home, app)?.ok_or("应用尚未安装")?;
    let path = desktop(&home, app);
    check_path(&home, &path)?;
    Ok(path)
}

// Read a regular member from Electron's ASAR pickle/JSON header without
// executing JavaScript or following ASAR links. All arithmetic is bounded.
fn asar_member<'a>(bytes: &'a [u8], member: &str) -> Result<&'a [u8], String> {
    if bytes.len() < 16 || !safe_relative(Path::new(member)) {
        return Err("ASAR 文件头或成员路径无效".into());
    }
    let u32_at = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap()) as usize;
    if u32_at(0) != 4 {
        return Err("ASAR pickle 无效".into());
    }
    let data_start = 8usize.checked_add(u32_at(4)).ok_or("ASAR 偏移溢出")?;
    let json_end = 16usize
        .checked_add(u32_at(12))
        .ok_or("ASAR JSON 偏移溢出")?;
    if json_end > data_start || data_start > bytes.len() {
        return Err("ASAR 文件头越界".into());
    }
    let header: serde_json::Value = serde_json::from_slice(&bytes[16..json_end]).map_err(err)?;
    let mut node = &header;
    for part in member.split('/') {
        node = &node["files"][part];
    }
    if node.get("link").is_some() || node["unpacked"].as_bool() == Some(true) {
        return Err("ASAR 成员不是内嵌普通文件".into());
    }
    let offset: usize = node["offset"]
        .as_str()
        .ok_or("ASAR 成员缺少偏移")?
        .parse()
        .map_err(err)?;
    let size: usize = node["size"]
        .as_u64()
        .ok_or("ASAR 成员缺少大小")?
        .try_into()
        .map_err(err)?;
    let start = data_start.checked_add(offset).ok_or("ASAR 成员偏移溢出")?;
    let end = start.checked_add(size).ok_or("ASAR 成员大小溢出")?;
    bytes.get(start..end).ok_or_else(|| "ASAR 成员越界".into())
}
fn validate_version(bytes: &[u8], version: &str) -> Result<(), String> {
    let json: serde_json::Value = serde_json::from_slice(bytes).map_err(err)?;
    if json["version"].as_str() != Some(version) {
        return Err("归档内版本与签名 feed 不一致".into());
    }
    Ok(())
}

/// Validate the entire stream (including gzip CRC) and optional extraction.
/// Links must be relative and remain inside the archive root. Hard links,
/// devices, FIFOs, duplicate members, and writes through symlink parents fail.
fn walk_archive(
    app: &Application,
    path: &Path,
    version: &str,
    output: Option<&Path>,
    cancel: &crate::source_engine::CancelCheck,
) -> Result<(), String> {
    let (executable, icon_path) = spec(app)?;
    let limit = MAX_UNPACKED + 100 * 1024 * 1024;
    let mut archive = tar::Archive::new(GzDecoder::new(File::open(path).map_err(err)?).take(limit));
    let mut seen = HashSet::new();
    let mut symlinks = HashSet::new();
    let mut unpacked = 0u64;
    let mut found_executable = false;
    let mut found_version = false;
    let mut found_icon = false;
    let top = Path::new(executable)
        .components()
        .next()
        .ok_or("缺少归档根目录")?
        .as_os_str();
    let asar = Path::new(executable)
        .parent()
        .ok_or("缺少归档根目录")?
        .join("resources/app.asar");
    let manifest = Path::new(executable)
        .parent()
        .ok_or("缺少归档根目录")?
        .join("resources/app/package.json");
    for entry in archive.entries().map_err(err)? {
        if cancel() {
            return Err(crate::source_engine::CANCELED_ERROR.into());
        }
        let mut entry = entry.map_err(err)?;
        let path = entry.path().map_err(err)?.into_owned();
        if !safe_relative(&path)
            || path.components().next().map(|c| c.as_os_str()) != Some(top)
            || !seen.insert(path.clone())
            || seen.len() > MAX_ENTRIES
        {
            return Err("归档包含越界、重复路径或过多文件".into());
        }
        if path.ancestors().skip(1).any(|p| symlinks.contains(p)) {
            return Err("归档试图通过符号链接写入".into());
        }
        let kind = entry.header().entry_type();
        let size = entry.size();
        unpacked = unpacked.checked_add(size).ok_or("归档大小溢出")?;
        if unpacked > MAX_UNPACKED {
            return Err("归档解压大小超过 4 GiB".into());
        }
        if kind.is_symlink() {
            let target = entry.link_name().map_err(err)?.ok_or("链接没有目标")?;
            // Reject parent traversal even when lexically contained: a prior
            // symlink in the target can change what `..` would resolve to.
            if target.as_os_str().is_empty()
                || target
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            {
                return Err("归档链接目标必须是无父目录跳转的相对路径".into());
            }
            // No earlier descendants either: a link may not replace a directory.
            if seen.iter().any(|p| p != &path && p.starts_with(&path)) {
                return Err("归档链接覆盖目录".into());
            }
            symlinks.insert(path.clone());
            if let Some(out) = output {
                let dest = out.join(&path);
                fs::create_dir_all(dest.parent().ok_or("缺少父目录")?).map_err(err)?;
                std::os::unix::fs::symlink(target.as_ref(), dest).map_err(err)?;
            }
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            return Err("归档包含不支持的特殊文件".into());
        }
        if kind.is_dir() {
            if let Some(out) = output {
                fs::create_dir_all(out.join(&path)).map_err(err)?;
            }
            continue;
        }
        let mut prefix = Vec::new();
        if path == Path::new(executable) {
            let mut header = [0u8; 20];
            entry.read_exact(&mut header).map_err(err)?;
            let machine = match app.architecture.as_str() {
                "amd64" => 62,
                "arm64" => 183,
                _ => return Err("不支持的归档架构".into()),
            };
            if &header[..4] != b"\x7fELF"
                || header[4] != 2
                || header[5] != 1
                || u16::from_le_bytes([header[18], header[19]]) != machine
            {
                return Err("归档程序 ELF 架构不匹配".into());
            }
            if entry.header().mode().map_err(err)? & 0o111 == 0 {
                return Err("归档主程序不可执行".into());
            }
            prefix.extend_from_slice(&header);
            found_executable = true;
        }
        if path == manifest {
            if size > 1024 * 1024 {
                return Err("归档版本文件过大".into());
            }
            entry.read_to_end(&mut prefix).map_err(err)?;
            validate_version(&prefix, version)?;
            found_version = true;
        }
        if path == asar {
            if size > 64 * 1024 * 1024 {
                return Err("ASAR 文件超过 64 MiB".into());
            }
            copy_checked(&mut entry, &mut prefix, cancel)?;
            validate_version(asar_member(&prefix, "package.json")?, version)?;
            found_version = true;
            if let Some((archive_path, member)) = icon_path.split_once(':') {
                if Path::new(archive_path) != path {
                    return Err("ASAR 图标归档路径不一致".into());
                }
                let icon = asar_member(&prefix, member)?;
                if icon.len() > 4 * 1024 * 1024 || !icon.starts_with(b"\x89PNG\r\n\x1a\n") {
                    return Err("ASAR 图标不是有效 PNG".into());
                }
                if let Some(out) = output {
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o644)
                        .open(out.join(".umanager-icon.png"))
                        .map_err(err)?;
                    file.write_all(icon).map_err(err)?;
                }
                found_icon = true;
            }
        }
        if path == Path::new(icon_path) {
            found_icon = true;
        }
        if let Some(out) = output {
            let dest = out.join(&path);
            fs::create_dir_all(dest.parent().ok_or("缺少父目录")?).map_err(err)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&dest)
                .map_err(err)?;
            file.write_all(&prefix).map_err(err)?;
            copy_checked(&mut entry, &mut file, cancel)?;
            // Drop all setuid/setgid bits. No root chrome-sandbox configuration.
            fs::set_permissions(
                dest,
                fs::Permissions::from_mode(entry.header().mode().map_err(err)? & 0o777),
            )
            .map_err(err)?;
        } else {
            copy_checked(&mut entry, &mut std::io::sink(), cancel)?;
        }
    }
    // Consume the gzip trailer too, so truncation/CRC errors are never ignored.
    let mut decoder = archive.into_inner();
    copy_checked(&mut decoder, &mut std::io::sink(), cancel)?;
    if decoder.limit() == 0 {
        return Err("归档流超过解压上限".into());
    }
    if !found_executable || !found_version || !found_icon {
        return Err("归档缺少主程序、版本文件或图标".into());
    }
    Ok(())
}
fn copy_checked(
    reader: &mut impl Read,
    writer: &mut impl Write,
    cancel: &crate::source_engine::CancelCheck,
) -> Result<(), String> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if cancel() {
            return Err(crate::source_engine::CANCELED_ERROR.into());
        }
        let n = reader.read(&mut buffer).map_err(err)?;
        if n == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..n]).map_err(err)?;
    }
}
pub fn validate_archive_cancellable(
    app: &Application,
    path: &Path,
    version: &str,
    cancel: &crate::source_engine::CancelCheck,
) -> Result<(), String> {
    walk_archive(app, path, version, None, cancel)
}
#[cfg(test)]
pub fn validate_archive(app: &Application, path: &Path, version: &str) -> Result<(), String> {
    walk_archive(
        app,
        path,
        version,
        None,
        &crate::source_engine::never_cancel(),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Payload {
    schema_version: u32,
    application_id: String,
    remove: bool,
    version: String,
    sha256: String,
    installed_version: Option<String>,
    installed_sha256: Option<String>,
    uid: u32,
    created_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchivePlan {
    pub plan_id: String,
    payload: Payload,
    install_path: String,
}
fn plan_hash(payload: &Payload) -> Result<String, String> {
    Ok(digest(&serde_json::to_vec(payload).map_err(err)?))
}
fn cache(handle: &tauri::AppHandle) -> Result<PathBuf, String> {
    handle.path().app_cache_dir().map_err(err)
}
fn plan_path(cache: &Path, id: &str) -> Result<PathBuf, String> {
    if !crate::zip_deb::valid_digest(id) {
        return Err("归档计划 ID 无效".into());
    }
    Ok(cache.join("archive-plans").join(format!("{id}.json")))
}

#[tauri::command]
pub async fn prepare_archive_operation(
    app: tauri::AppHandle,
    application_id: String,
    remove: bool,
) -> Result<ArchivePlan, String> {
    let home = home()?;
    let catalog = crate::feed::effective_catalog().await?;
    let application = catalog
        .by_application_id(&application_id)
        .ok_or("未受管的归档应用")?;
    let root = root(&home, application)?;
    if remove && !application.removable {
        return Err("该应用不允许卸载".into());
    }
    let installed = state(&home, application)?;
    let cache = cache(&app)?;
    let (version, sha256) = if remove {
        let s = installed.as_ref().ok_or("应用尚未安装")?;
        (s.version.clone(), s.sha256.clone())
    } else {
        let verified = match crate::source_engine::verify_cached(application, &cache).await {
            Ok(verified) => verified,
            Err(error) => {
                if error.starts_with(crate::source_engine::CACHE_REVALIDATION_ERROR) {
                    if let Some(job_id) = app
                        .state::<crate::download_queue::DownloadQueue>()
                        .ready_job_id(&application_id)
                    {
                        crate::download_queue::invalidate_ready(&app, &job_id, error.clone());
                    }
                }
                return Err(error);
            }
        };
        (verified.version, verified.actual_sha256)
    };
    validate_desktop(&home, application)?;
    let payload = Payload {
        schema_version: 1,
        application_id,
        remove,
        version,
        sha256,
        installed_version: installed.as_ref().map(|s| s.version.clone()),
        installed_sha256: installed.map(|s| s.sha256),
        uid: fs::metadata(&home).map_err(err)?.uid(),
        created_at_unix_seconds: now(),
        expires_at_unix_seconds: now() + 900,
    };
    let plan = ArchivePlan {
        plan_id: plan_hash(&payload)?,
        payload,
        install_path: root.to_string_lossy().into_owned(),
    };
    let path = plan_path(&cache, &plan.plan_id)?;
    check_path(&home, &path)?;
    fs::create_dir_all(path.parent().ok_or("计划目录无效")?).map_err(err)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(path)
        .map_err(err)?;
    file.write_all(&serde_json::to_vec(&plan).map_err(err)?)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    Ok(plan)
}

fn validate_plan(plan: &ArchivePlan, id: &str, uid: u32, now: u64) -> Result<(), String> {
    let p = &plan.payload;
    if plan.plan_id != id
        || plan_hash(p)? != id
        || p.schema_version != 1
        || p.uid != uid
        || p.created_at_unix_seconds > now
        || p.expires_at_unix_seconds <= now
        || p.expires_at_unix_seconds
            .checked_sub(p.created_at_unix_seconds)
            != Some(900)
    {
        return Err("归档计划已过期、损坏或不属于当前用户，请重新准备".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn execute_archive_operation(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    let _guard = crate::operation_plan::MUTATION_LOCK.lock().await;
    let home = home()?;
    let cache = cache(&app)?;
    let path = plan_path(&cache, &plan_id)?;
    check_path(&home, &path)?;
    let metadata = fs::metadata(&path).map_err(err)?;
    if metadata.mode() & 0o222 != 0 || metadata.len() > 16 * 1024 {
        return Err("归档计划不是只读或大小异常".into());
    }
    let plan: ArchivePlan = serde_json::from_slice(&fs::read(&path).map_err(err)?).map_err(err)?;
    let p = &plan.payload;
    validate_plan(
        &plan,
        &plan_id,
        fs::metadata(&home).map_err(err)?.uid(),
        now(),
    )?;
    let catalog = crate::feed::effective_catalog().await?;
    let application = catalog
        .by_application_id(&p.application_id)
        .ok_or("归档应用已不在签名目录")?
        .clone();
    let root = root(&home, &application)?;
    if plan.install_path != root.to_string_lossy() {
        return Err("计划安装目录不一致".into());
    }
    let installed = state(&home, &application)?;
    if installed.as_ref().map(|s| &s.version) != p.installed_version.as_ref()
        || installed.as_ref().map(|s| &s.sha256) != p.installed_sha256.as_ref()
    {
        return Err("安装状态已变化，请重新准备计划".into());
    }
    let target = desktop(&home, &application);
    validate_desktop(&home, &application)?;
    if p.remove {
        if !application.removable {
            return Err("该应用不允许卸载".into());
        }
        if p.installed_version.as_ref() != Some(&p.version)
            || p.installed_sha256.as_ref() != Some(&p.sha256)
        {
            return Err("卸载目标与安装记录不一致".into());
        }
        // Never remove vendor data outside this dedicated UManager app root.
        if target.exists() {
            fs::remove_file(target).map_err(err)?;
        }
        fs::remove_dir_all(root).map_err(err)?;
        return Ok("应用已卸载，个人数据已保留".into());
    }
    let verified = crate::source_engine::verify_cached(&application, &cache).await?;
    if verified.version != p.version || verified.actual_sha256 != p.sha256 {
        return Err("签名 feed 或安装包已变化，请重新准备计划".into());
    }
    let input = PathBuf::from(verified.plan.target_path);
    tauri::async_runtime::spawn_blocking(move || {
        install(&home, &root, &target, &application, &input, &plan.payload)
    })
    .await
    .map_err(err)??;
    Ok("应用已安装到当前用户目录，可从应用菜单打开".into())
}

fn validate_desktop(home: &Path, app: &Application) -> Result<(), String> {
    let path = desktop(home, app);
    check_path(home, &path)?;
    match fs::read_to_string(&path) {
        Ok(text)
            if !text
                .lines()
                .any(|line| line == format!("X-UManager-Application={}", app.application_id)) =>
        {
            Err("已存在同名非 UManager 桌面启动项，请先处理冲突".into())
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(err(e)),
    }
}

fn atomic_write(home: &Path, path: &Path, bytes: &[u8]) -> Result<(), String> {
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
fn desktop_quote(path: &Path) -> Result<String, String> {
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
fn install(
    home: &Path,
    root: &Path,
    desktop: &Path,
    app: &Application,
    input: &Path,
    p: &Payload,
) -> Result<(), String> {
    check_path(home, root)?;
    validate_desktop(home, app)?;
    fs::create_dir_all(root).map_err(err)?;
    let stage = root.join(format!(".stage-{}-{}", std::process::id(), now()));
    fs::create_dir(&stage).map_err(err)?;
    let result = (|| {
        let copied = stage.join("archive.tar.gz");
        let mut source = File::open(input).map_err(err)?.take(1024 * 1024 * 1024 + 1);
        let mut copy = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&copied)
            .map_err(err)?;
        let copied_bytes = std::io::copy(&mut source, &mut copy).map_err(err)?;
        if copied_bytes > 1024 * 1024 * 1024 {
            return Err("安装归档超过 1 GiB".into());
        }
        copy.sync_all().map_err(err)?;
        drop(copy);
        let mut reader = File::open(&copied).map_err(err)?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buffer).map_err(err)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        if format!("{:x}", hash.finalize()) != p.sha256 {
            return Err("安装前归档 SHA-256 复核失败".into());
        }
        let extracted = stage.join("files");
        fs::create_dir(&extracted).map_err(err)?;
        walk_archive(
            app,
            &copied,
            &p.version,
            Some(&extracted),
            &crate::source_engine::never_cancel(),
        )?;
        let destination = root.join(&p.sha256);
        check_path(home, &destination)?;
        if destination.exists() && p.installed_sha256.as_deref() == Some(&p.sha256) {
            return Err("该版本已经安装，无需重复安装".into());
        }
        if destination.exists() {
            // A previous failed attempt may have staged this exact immutable version.
            fs::remove_dir_all(&destination).map_err(err)?;
        }
        fs::rename(extracted, &destination).map_err(err)?;
        let (executable, icon) = spec(app)?;
        let name = app.display_name.replace(['\n', '\r', '\\'], " ");
        let icon = destination
            .join(if icon.contains(':') {
                ".umanager-icon.png"
            } else {
                icon
            })
            .to_string_lossy()
            .replace('\\', "\\\\");
        if icon.contains(['\n', '\r']) {
            return Err("图标路径包含控制字符".into());
        }
        let text = format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={} %U\nIcon={icon}\nTerminal=false\nCategories=Development;\nX-UManager-Application={}\n",
            desktop_quote(&destination.join(executable))?,
            app.application_id
        );
        fs::create_dir_all(desktop.parent().ok_or("启动项目录无效")?).map_err(err)?;
        let previous_desktop = fs::read(desktop).ok();
        atomic_write(home, desktop, text.as_bytes())?;
        if let Err(error) = atomic_write(
            home,
            &root.join("installed.json"),
            &serde_json::to_vec(&Installed {
                version: p.version.clone(),
                sha256: p.sha256.clone(),
            })
            .map_err(err)?,
        ) {
            let rollback = if let Some(bytes) = previous_desktop {
                atomic_write(home, desktop, &bytes)
            } else {
                fs::remove_file(desktop).map_err(err)
            };
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback) => format!("{error}；启动项回滚失败：{rollback}"),
            });
        }
        // Keep previous versions: running processes may still need their resources.
        Ok(())
    })();
    let _ = fs::remove_dir_all(stage);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    fn app() -> Application {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../feed-sources.json")).unwrap();
        serde_json::from_value(
            json["applications"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["applicationId"] == "antigravity")
                .unwrap()
                .clone(),
        )
        .unwrap()
    }
    fn fixture_app() -> Application {
        let mut app = app();
        if let SourceSpec::TarGz { icon_path, .. } = &mut app.source {
            *icon_path = "Antigravity-x64/resources/app/resources/linux/code.png".into();
        }
        app
    }
    fn temp() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "umanager-archive-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    fn fixture(root: &Path, version: &str, machine: u16, extra: Option<(&str, &str)>) -> PathBuf {
        let path = root.join("test.tar.gz");
        let mut tar = tar::Builder::new(GzEncoder::new(
            File::create(&path).unwrap(),
            Compression::fast(),
        ));
        let executable = spec(&app()).unwrap().0.to_owned();
        let mut elf = vec![0u8; 32];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[18..20].copy_from_slice(&machine.to_le_bytes());
        let manifest = format!("{{\"version\":\"{version}\"}}");
        for (name, bytes) in [
            (executable.as_str(), elf.as_slice()),
            (
                "Antigravity-x64/resources/app/package.json",
                manifest.as_bytes(),
            ),
            (
                "Antigravity-x64/resources/app/resources/linux/code.png",
                b"icon",
            ),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o4755);
            header.set_cksum();
            tar.append_data(&mut header, name, bytes).unwrap();
        }
        if let Some((name, target)) = extra {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o777);
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name(target).unwrap();
            header.set_cksum();
            tar.append_data(&mut header, name, std::io::empty())
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        path
    }
    #[test]
    fn validates_version_architecture_and_contained_links() {
        let root = temp();
        let path = fixture(
            &root,
            "2.29.0",
            62,
            Some(("Antigravity-x64/lib/link", "target")),
        );
        assert!(validate_archive(&fixture_app(), &path, "2.29.0").is_ok());
        assert!(validate_archive(&fixture_app(), &path, "2.30.0").is_err());
        fixture(&root, "2.29.0", 183, None);
        assert!(validate_archive(&fixture_app(), &path, "2.29.0").is_err());
        for target in [
            "../../../outside",
            "/etc/passwd",
            "../../outside",
            "target/../../outside",
        ] {
            fixture(
                &root,
                "2.29.0",
                62,
                Some(("Antigravity-x64/lib/link", target)),
            );
            assert!(
                validate_archive(&fixture_app(), &path, "2.29.0").is_err(),
                "{target}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn installs_without_running_code_and_drops_setuid_bits() {
        let home = temp();
        let input = fixture(&home, "2.29.0", 62, None);
        let hash = digest(&fs::read(&input).unwrap());
        let app = fixture_app();
        let root = root(&home, &app).unwrap();
        let desktop = desktop(&home, &app);
        let p = Payload {
            schema_version: 1,
            application_id: app.application_id.clone(),
            remove: false,
            version: "2.29.0".into(),
            sha256: hash.clone(),
            installed_version: None,
            installed_sha256: None,
            uid: fs::metadata(&home).unwrap().uid(),
            created_at_unix_seconds: now(),
            expires_at_unix_seconds: now() + 900,
        };
        install(&home, &root, &desktop, &app, &input, &p).unwrap();
        assert_eq!(state(&home, &app).unwrap().unwrap().version, "2.29.0");
        let executable = root.join(hash).join(spec(&app).unwrap().0);
        assert_eq!(fs::metadata(&executable).unwrap().mode() & 0o7777, 0o755);
        assert!(
            fs::read_to_string(desktop)
                .unwrap()
                .contains(&desktop_quote(&executable).unwrap())
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn rejects_symlink_ancestors_and_corrupt_streams() {
        let root = temp();
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".local")).unwrap();
        assert!(super::root(&root, &app()).is_err());
        let input = fixture(&root, "2.29.0", 62, None);
        let mut bytes = fs::read(&input).unwrap();
        bytes.truncate(bytes.len() - 6);
        fs::write(&input, bytes).unwrap();
        assert!(validate_archive(&fixture_app(), &input, "2.29.0").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_traversal_special_files_duplicates_and_link_parents() {
        for attack in [
            "traversal",
            "device",
            "hardlink",
            "duplicate",
            "link-parent",
        ] {
            let root = temp();
            let path = root.join("bad.tar.gz");
            let mut tar = tar::Builder::new(GzEncoder::new(
                File::create(&path).unwrap(),
                Compression::fast(),
            ));
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(0);
            if attack == "traversal" {
                let name = b"Antigravity-x64/../../escape";
                header.as_mut_bytes()[..name.len()].copy_from_slice(name);
                header.set_cksum();
                tar.append(&header, std::io::empty()).unwrap();
            } else {
                let name = "Antigravity-x64/dir";
                if attack == "device" {
                    header.set_entry_type(tar::EntryType::Char);
                }
                if attack == "hardlink" {
                    header.set_entry_type(tar::EntryType::Link);
                    header.set_link_name("Antigravity-x64/target").unwrap();
                }
                if attack == "link-parent" {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_link_name("target").unwrap();
                }
                header.set_cksum();
                tar.append_data(&mut header, name, std::io::empty())
                    .unwrap();
                if attack == "duplicate" {
                    tar.append_data(&mut header, name, std::io::empty())
                        .unwrap();
                }
                if attack == "link-parent" {
                    let mut child = tar::Header::new_gnu();
                    child.set_mode(0o644);
                    child.set_size(0);
                    child.set_cksum();
                    tar.append_data(&mut child, "Antigravity-x64/dir/file", std::io::empty())
                        .unwrap();
                }
            }
            tar.into_inner().unwrap().finish().unwrap();
            let output = root.join("output");
            fs::create_dir(&output).unwrap();
            assert!(
                walk_archive(
                    &fixture_app(),
                    &path,
                    "2.29.0",
                    Some(&output),
                    &crate::source_engine::never_cancel()
                )
                .is_err(),
                "{attack}"
            );
            assert!(!root.join("escape").exists());
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn asar_reads_only_bounded_regular_members_and_cancellation_stops_validation() {
        let manifest = b"{\"version\":\"2.29.0\"}";
        let json = serde_json::to_vec(
            &serde_json::json!({"files":{"package.json":{"offset":"0","size":manifest.len()}}}),
        )
        .unwrap();
        let mut bytes = Vec::new();
        for n in [
            4u32,
            (json.len() + 8) as u32,
            (json.len() + 4) as u32,
            json.len() as u32,
        ] {
            bytes.extend(n.to_le_bytes());
        }
        bytes.extend(&json);
        bytes.extend(manifest);
        assert_eq!(asar_member(&bytes, "package.json").unwrap(), manifest);
        assert!(asar_member(&bytes, "missing.json").is_err());
        assert!(asar_member(&bytes[..bytes.len() - 1], "package.json").is_err());
        assert!(asar_member(&bytes, "../package.json").is_err());
        let root = temp();
        let input = fixture(&root, "2.29.0", 62, None);
        let cancel: crate::source_engine::CancelCheck = std::sync::Arc::new(|| true);
        assert_eq!(
            validate_archive_cancellable(&fixture_app(), &input, "2.29.0", &cancel).unwrap_err(),
            crate::source_engine::CANCELED_ERROR
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn plan_checks_integrity_owner_and_exact_lifetime() {
        let payload = Payload {
            schema_version: 1,
            application_id: "antigravity".into(),
            remove: false,
            version: "2.29.0".into(),
            sha256: "a".repeat(64),
            installed_version: None,
            installed_sha256: None,
            uid: 1000,
            created_at_unix_seconds: 100,
            expires_at_unix_seconds: 1000,
        };
        let mut plan = ArchivePlan {
            plan_id: plan_hash(&payload).unwrap(),
            payload,
            install_path: "/home/example/.local/share/umanager/apps/antigravity".into(),
        };
        assert!(validate_plan(&plan, &plan.plan_id, 1000, 500).is_ok());
        assert!(validate_plan(&plan, &plan.plan_id, 1001, 500).is_err());
        assert!(validate_plan(&plan, &plan.plan_id, 1000, 99).is_err());
        assert!(validate_plan(&plan, &plan.plan_id, 1000, 1000).is_err());
        plan.payload.remove = true;
        assert!(validate_plan(&plan, &plan.plan_id, 1000, 500).is_err());
        plan.payload.expires_at_unix_seconds += 1;
        plan.plan_id = plan_hash(&plan.payload).unwrap();
        assert!(validate_plan(&plan, &plan.plan_id, 1000, 500).is_err());
    }
    #[test]
    fn update_rolls_back_launcher_if_state_commit_fails() {
        let home = temp();
        let app = fixture_app();
        let input = fixture(&home, "2.29.0", 62, None);
        let first_hash = digest(&fs::read(&input).unwrap());
        let root = root(&home, &app).unwrap();
        let target = desktop(&home, &app);
        let mut payload = Payload {
            schema_version: 1,
            application_id: app.application_id.clone(),
            remove: false,
            version: "2.29.0".into(),
            sha256: first_hash.clone(),
            installed_version: None,
            installed_sha256: None,
            uid: fs::metadata(&home).unwrap().uid(),
            created_at_unix_seconds: now(),
            expires_at_unix_seconds: now() + 900,
        };
        install(&home, &root, &target, &app, &input, &payload).unwrap();
        let old_desktop = fs::read(&target).unwrap();
        fixture(&home, "2.30.0", 62, None);
        payload.installed_version = Some(payload.version.clone());
        payload.installed_sha256 = Some(payload.sha256.clone());
        payload.version = "2.30.0".into();
        payload.sha256 = digest(&fs::read(&input).unwrap());
        let blocker = root.join("installed.umanager-new");
        fs::write(&blocker, b"existing").unwrap();
        assert!(install(&home, &root, &target, &app, &input, &payload).is_err());
        assert_eq!(fs::read(&target).unwrap(), old_desktop);
        assert_eq!(state(&home, &app).unwrap().unwrap().version, "2.29.0");
        fs::remove_file(blocker).unwrap();
        install(&home, &root, &target, &app, &input, &payload).unwrap();
        assert_eq!(state(&home, &app).unwrap().unwrap().version, "2.30.0");
        assert!(
            root.join(first_hash).exists(),
            "running previous version remains available"
        );
        fs::remove_dir_all(home).unwrap();
    }
    #[test]
    #[ignore = "requires an explicitly downloaded official archive"]
    fn validates_official_archive() {
        let path = PathBuf::from(
            std::env::var_os("UMANAGER_TEST_ARCHIVE").expect("set UMANAGER_TEST_ARCHIVE"),
        );
        let version = std::env::var("UMANAGER_TEST_VERSION").expect("set UMANAGER_TEST_VERSION");
        let app = app();
        validate_archive(&app, &path, &version).unwrap();
        let home = temp();
        let root = root(&home, &app).unwrap();
        let target = desktop(&home, &app);
        let hash = digest(&fs::read(&path).unwrap());
        let payload = Payload {
            schema_version: 1,
            application_id: app.application_id.clone(),
            remove: false,
            version: version.clone(),
            sha256: hash.clone(),
            installed_version: None,
            installed_sha256: None,
            uid: fs::metadata(&home).unwrap().uid(),
            created_at_unix_seconds: now(),
            expires_at_unix_seconds: now() + 900,
        };
        install(&home, &root, &target, &app, &path, &payload).unwrap();
        assert_eq!(state(&home, &app).unwrap().unwrap().version, version);
        assert!(root.join(hash).join(".umanager-icon.png").is_file());
        assert!(
            fs::read_to_string(target)
                .unwrap()
                .contains("X-UManager-Application=antigravity")
        );
        fs::remove_dir_all(home).unwrap();
    }
}
