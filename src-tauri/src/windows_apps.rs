//! User-level Windows applications. The signed central feed authorizes installers;
//! no scraping, shell interpolation, privileged helper, or registry command execution.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};

const EXE: &str = "drive_c/Program Files (x86)/WXWork/WXWork.exe";
const UNINSTALL: &str = "drive_c/Program Files (x86)/WXWork/Uninstall.exe";
const MAX_INSTALLER: u64 = 1024 * 1024 * 1024;
const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wecom-titlebar.exe"));
static BUSY: AtomicBool = AtomicBool::new(false);
static PLANS: OnceLock<Mutex<HashMap<String, Prepared>>> = OnceLock::new();
struct Busy;
impl Busy {
    fn acquire() -> Result<Self, String> {
        BUSY.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Self)
            .map_err(|_| "另一个 Windows 应用操作正在进行".into())
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsRelease {
    pub display_name: String,
    pub profile: String,
    pub version: String,
    pub download_url: String,
    pub download_hosts: Vec<String>,
    pub size: u64,
    pub sha256: String,
}
impl WindowsRelease {
    pub fn validate(&self) -> Result<(), String> {
        let catalog: serde_json::Value =
            serde_json::from_str(umanager_catalog::CATALOG_JSON).map_err(error)?;
        let allowed_hosts = catalog["windowsCompatibilityProfiles"][&self.profile]["downloadHosts"]
            .as_array()
            .ok_or("内置目录未授权此 Windows 兼容配置")?;
        if self.profile != "wecom-v1"
            || version_parts(&self.version).is_none()
            || self.size < 1024
            || self.size > MAX_INSTALLER
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            || self.download_hosts.is_empty()
            || self
                .download_hosts
                .iter()
                .any(|h| !allowed_hosts.iter().any(|v| v.as_str() == Some(h.as_str())))
        {
            return Err("企业微信签名元数据无效或需要新版兼容配置".into());
        }
        let url = url::Url::parse(&self.download_url).map_err(error)?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port_or_known_default() != Some(443)
            || !url
                .host_str()
                .is_some_and(|h| self.download_hosts.iter().any(|x| x == h))
            || url.path() != format!("/wework/work_weixin/WeCom_{}.exe", self.version)
        {
            return Err("企业微信下载地址不在授权范围内".into());
        }
        Ok(())
    }
}

fn default_font_antialiasing() -> String {
    "default".into()
}
fn default_font_hinting() -> String {
    "default".into()
}
fn default_font_link() -> bool {
    true
}
fn default_virtual_desktop() -> String {
    "off".into()
}
fn default_color_depth() -> u32 {
    32
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WineSettings {
    pub wine_binary: String,
    pub windows_version: String,
    pub dpi: u32,
    pub graphics_driver: String,
    pub chinese_font: String,
    pub titlebar_fix: bool,
    #[serde(default = "default_font_antialiasing")]
    pub font_antialiasing: String,
    #[serde(default = "default_font_hinting")]
    pub font_hinting: String,
    #[serde(default = "default_font_link")]
    pub font_link: bool,
    #[serde(default = "default_virtual_desktop")]
    pub virtual_desktop: String,
    #[serde(default = "default_color_depth")]
    pub color_depth: u32,
}
impl Default for WineSettings {
    fn default() -> Self {
        serde_json::from_str(include_str!("../resources/windows/wecom-defaults.json"))
            .expect("built-in Wine defaults")
    }
}
impl WineSettings {
    fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.wine_binary.as_str(),
            "/usr/bin/wine"
                | "/opt/wine-devel/bin/wine"
                | "/opt/wine-stable/bin/wine"
                | "/opt/wine-staging/bin/wine"
        ) || !matches!(self.windows_version.as_str(), "win10" | "win11")
            || !(96..=288).contains(&self.dpi)
            || !matches!(self.graphics_driver.as_str(), "x11" | "wayland")
            || self.chinese_font != "Noto Sans CJK SC"
            || !matches!(
                self.font_antialiasing.as_str(),
                "default" | "gray" | "rgb" | "bgr"
            )
            || !matches!(
                self.font_hinting.as_str(),
                "default" | "none" | "light" | "medium" | "full"
            )
            || !matches!(
                self.virtual_desktop.as_str(),
                "off" | "1280x720" | "1920x1080" | "2560x1440"
            )
            || !matches!(self.color_depth, 16 | 24 | 32)
        {
            return Err("不支持的 Wine 配置".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
struct Paths {
    home: PathBuf,
    root: PathBuf,
    prefix: PathBuf,
}
impl Paths {
    fn new() -> Result<Self, String> {
        if fs::metadata("/proc/self").map_err(error)?.uid() == 0 {
            return Err("Windows 应用必须以普通用户运行，不能通过 root 或 sudo 管理".into());
        }
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("无法定位用户目录")?);
        Self::at(home)
    }
    fn at(home: PathBuf) -> Result<Self, String> {
        let home = home.canonicalize().map_err(error)?;
        let root = home.join(".local/share/umanager/windows");
        let existing = home.join(".local/share/wineprefixes/wecom");
        let prefix = if existing.is_dir() {
            existing
        } else {
            root.join("wecom")
        };
        let result = Self { home, root, prefix };
        result.check()?;
        Ok(result)
    }
    fn check(&self) -> Result<(), String> {
        // The home itself is canonical; reject symlinks below it, including
        // installer/uninstaller files. Never follow an imported prefix elsewhere.
        for target in [
            &self.prefix,
            &self.root,
            &self.prefix.join(EXE),
            &self.prefix.join(UNINSTALL),
        ] {
            check_local_path(&self.home, target)?;
        }
        Ok(())
    }
    fn settings(&self) -> Result<WineSettings, String> {
        let path = self.root.join("settings.json");
        check_local_path(&self.home, &path)?;
        match fs::read(&path) {
            Ok(bytes) => {
                let s: WineSettings = serde_json::from_slice(&bytes).map_err(error)?;
                s.validate()?;
                Ok(s)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WineSettings::default()),
            Err(e) => Err(error(e)),
        }
    }
}
fn check_local_path(home: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(home).map_err(|_| "路径不在用户目录内")?;
    let owner = fs::metadata(home).map_err(error)?.uid();
    let mut current = home.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("路径包含非正常分量".into());
        }
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() || m.uid() != owner => {
                return Err(format!(
                    "路径必须属于当前用户且不能是符号链接：{}",
                    current.display()
                ));
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(error(e)),
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsState {
    installed: bool,
    installed_version: Option<String>,
    candidate_version: Option<String>,
    update_available: bool,
    prefix: String,
    wine_version: Option<String>,
    settings: WineSettings,
    running: bool,
    font_available: bool,
    feed_error: Option<String>,
    busy: bool,
}
pub async fn state() -> Result<WindowsState, String> {
    let paths = Paths::new()?;
    let settings = paths.settings()?;
    let (release, feed_error) = match release().await {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(e)),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let installed_version = installed_version(&paths.prefix);
        Ok(WindowsState {
            installed: paths.prefix.join(EXE).is_file(),
            update_available: release
                .as_ref()
                .zip(installed_version.as_ref())
                .is_some_and(|(r, v)| version_parts(&r.version) > version_parts(v)),
            installed_version,
            candidate_version: release.map(|r| r.version),
            prefix: paths.prefix.display().to_string(),
            wine_version: wine_version(&settings),
            running: running(&paths.prefix),
            font_available: font_available(),
            settings,
            feed_error,
            busy: BUSY.load(Ordering::SeqCst),
        })
    })
    .await
    .map_err(error)?
}
async fn release() -> Result<WindowsRelease, String> {
    let catalog = umanager_catalog::Catalog::load()?;
    let r = crate::feed::load(&catalog)
        .await?
        .windows_applications
        .get("wecom")
        .cloned()
        .ok_or("签名软件源尚未提供企业微信 Windows 安装包；更新软件源后重试")?;
    r.validate()?;
    Ok(r)
}
fn wine_version(settings: &WineSettings) -> Option<String> {
    Command::new(&settings.wine_binary)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}
fn font_available() -> bool {
    Path::new("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc").is_file()
}
fn version_parts(v: &str) -> Option<Vec<u32>> {
    let parts: Vec<_> = v.split('.').collect();
    if parts.len() != 4
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    parts.into_iter().map(|p| p.parse().ok()).collect()
}
fn pe_version(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"MZ") {
        return None;
    }
    let at = bytes
        .windows(8)
        .position(|s| s == [0xbd, 0x04, 0xef, 0xfe, 0, 0, 1, 0])?;
    let hi = u32::from_le_bytes(bytes.get(at + 8..at + 12)?.try_into().ok()?);
    let lo = u32::from_le_bytes(bytes.get(at + 12..at + 16)?.try_into().ok()?);
    Some(format!(
        "{}.{}.{}.{}",
        hi >> 16,
        hi & 65535,
        lo >> 16,
        lo & 65535
    ))
}
fn installed_version(prefix: &Path) -> Option<String> {
    let path = prefix.join(EXE);
    // The WeCom main executable is ~257 MB, far above a naive 128 MB guard, so
    // cap at 1 GiB. The version signature sits near the end of the file; reading
    // it once is cheap (~0.2 s) and only happens on a status refresh.
    if fs::metadata(&path).ok()?.len() > 1024 * 1024 * 1024 {
        return None;
    }
    pe_version(&fs::read(path).ok()?)
}
fn running(prefix: &Path) -> bool {
    let expected = format!("WINEPREFIX={}", prefix.display());
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            if !entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
            {
                return false;
            }
            let cmd = fs::read(entry.path().join("cmdline")).unwrap_or_default();
            let matching = cmd.split(|b| *b == 0).any(|arg| {
                let text = String::from_utf8_lossy(arg)
                    .trim_matches('"')
                    .replace('\\', "/")
                    .to_ascii_lowercase();
                text.ends_with("/wxwork.exe") || text == "wxwork.exe"
            });
            matching
                && fs::read(entry.path().join("environ"))
                    .unwrap_or_default()
                    .split(|b| *b == 0)
                    .any(|v| v == expected.as_bytes())
        })
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Install,
    Update,
    Uninstall,
    Configure,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub plan_id: String,
    action: Action,
    prefix: String,
    installed_version: Option<String>,
    target_version: Option<String>,
    settings: WineSettings,
    expires_at: u64,
    download_size: Option<u64>,
    sha256: Option<String>,
}
#[derive(Clone)]
struct Prepared {
    plan: Plan,
    paths: Paths,
    installer: Option<PathBuf>,
    release: Option<WindowsRelease>,
    fingerprint: String,
}
fn fingerprint(paths: &Paths) -> Result<String, String> {
    let mut hasher = Sha256::new();
    for path in [
        paths.prefix.join(EXE),
        paths.prefix.join(UNINSTALL),
        paths.root.join("settings.json"),
    ] {
        if path.exists() {
            hasher.update(fs::read(path).map_err(error)?);
        }
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn progress(app: &tauri::AppHandle, message: &str) {
    let _ = app.emit("windows-progress", message);
}
// Structured progress, same payload as Debian package downloads so the card ring and
// drawer progress bar behave identically to ordinary software.
fn download_progress(
    app: &tauri::AppHandle,
    phase: &'static str,
    transferred: u64,
    total: u64,
    bytes_per_second: u64,
) {
    let _ = app.emit(
        "windows-download-progress",
        crate::source_engine::DownloadProgress {
            package_name: "wecom".into(),
            phase,
            transferred_bytes: transferred,
            total_bytes: total,
            bytes_per_second,
        },
    );
}

pub async fn prepare(
    app: tauri::AppHandle,
    action: Action,
    settings: WineSettings,
) -> Result<Plan, String> {
    let _busy = Busy::acquire()?;
    settings.validate()?;
    let paths = Paths::new()?;
    if wine_version(&settings).is_none() {
        return Err("请先在软件商店安装 Wine，或选择已安装的 Wine 运行器".into());
    }
    if running(&paths.prefix) {
        return Err("请先从托盘退出企业微信，再进行安装、更新、卸载或配置".into());
    }
    let installed = paths.prefix.join(EXE).is_file();
    match action {
        Action::Install if installed => return Err("企业微信已安装，请使用更新".into()),
        Action::Update | Action::Uninstall if !installed => {
            return Err("未检测到企业微信安装".into());
        }
        Action::Configure if !paths.prefix.is_dir() => {
            return Err("请先安装企业微信以创建 Wine 环境".into());
        }
        _ => (),
    }
    if action != Action::Uninstall && !font_available() {
        return Err("缺少中文字体，请先安装 Ubuntu 的 fonts-noto-cjk 软件包".into());
    }
    let original_fingerprint = fingerprint(&paths)?;
    let current = installed_version(&paths.prefix);
    let release = if matches!(action, Action::Install | Action::Update) {
        let r = release().await?;
        if action == Action::Update {
            let version = current
                .as_ref()
                .and_then(|v| version_parts(v))
                .ok_or("无法识别当前版本，不能安全判断更新方向")?;
            if version_parts(&r.version).ok_or("候选版本无效")? <= version {
                return Err("签名软件源没有更高版本，已阻止重复更新或降级".into());
            }
        }
        Some(r)
    } else {
        None
    };
    let installer = if let Some(r) = &release {
        progress(&app, "正在下载并校验官方 Windows 安装包…");
        Some(download(&paths, r, &app).await?)
    } else {
        None
    };
    paths.check()?;
    let mut plan = Plan {
        plan_id: String::new(),
        action,
        prefix: paths.prefix.display().to_string(),
        installed_version: current,
        target_version: release.as_ref().map(|r| r.version.clone()),
        settings,
        expires_at: now() + 900,
        download_size: release.as_ref().map(|r| r.size),
        sha256: release.as_ref().map(|r| r.sha256.clone()),
    };
    let fingerprint = fingerprint(&paths)?;
    if fingerprint != original_fingerprint {
        return Err("下载期间安装或配置已改变，请重新复核".into());
    }
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(&plan).map_err(error)?);
    hasher.update(&fingerprint);
    plan.plan_id = format!("{:x}", hasher.finalize());
    let prepared = Prepared {
        plan: plan.clone(),
        paths,
        installer,
        release,
        fingerprint,
    };
    let mut plans = PLANS.get_or_init(Mutex::default).lock().map_err(error)?;
    plans.retain(|_, p| p.plan.expires_at > now());
    if plans.len() >= 16 {
        plans.clear();
    }
    plans.insert(plan.plan_id.clone(), prepared);
    progress(&app, "复核完成，请确认操作。安装与卸载将打开官方向导。");
    Ok(plan)
}

async fn download(
    paths: &Paths,
    release: &WindowsRelease,
    app: &tauri::AppHandle,
) -> Result<PathBuf, String> {
    release.validate()?;
    fs::create_dir_all(&paths.root).map_err(error)?;
    fs::set_permissions(&paths.root, fs::Permissions::from_mode(0o700)).map_err(error)?;
    let path = paths.root.join(format!("installer-{}.exe", release.sha256));
    check_local_path(&paths.home, &path)?;
    if path.exists() && verify_file(&path, release).is_ok() {
        return Ok(path);
    }
    if path.exists() {
        fs::remove_file(&path).map_err(error)?;
    }
    let partial = paths.root.join("installer.partial");
    check_local_path(&paths.home, &partial)?;
    if partial.exists() {
        fs::remove_file(&partial).map_err(error)?;
    }
    let result = async {
        // 官方安装包接近 650 MB；15 分钟的总超时在普通带宽下会中途失败并丢弃已下载
        // 数据，所以给足 60 分钟，进度按 1% 上报以便界面持续可见。
        let client = crate::source_engine::restricted_client(
            &release.download_hosts,
            Duration::from_secs(3600),
        )?;
        progress(app, "正在下载官方安装包…");
        let mut response = client
            .get(&release.download_url)
            .send()
            .await
            .map_err(error)?
            .error_for_status()
            .map_err(error)?;
        if response.content_length().is_some_and(|n| n != release.size) {
            return Err("安装包长度与签名软件源不一致".into());
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)
            .map_err(error)?;
        let mut transferred = 0;
        let mut last_percent = 0;
        let started = std::time::Instant::now();
        download_progress(app, "downloading", 0, release.size, 0);
        while let Some(chunk) = response.chunk().await.map_err(error)? {
            transferred += chunk.len() as u64;
            if transferred > release.size {
                return Err("安装包超出签名软件源声明的大小".into());
            }
            file.write_all(&chunk).map_err(error)?;
            let percent = transferred * 100 / release.size;
            if percent >= last_percent + 1 {
                progress(
                    app,
                    &format!(
                        "正在下载官方安装包 {percent}%（{} MB）",
                        transferred / 1024 / 1024
                    ),
                );
                download_progress(
                    app,
                    "downloading",
                    transferred,
                    release.size,
                    transferred / started.elapsed().as_secs().max(1),
                );
                last_percent = percent;
            }
        }
        file.sync_all().map_err(error)?;
        progress(app, "正在校验安装包大小与 SHA-256…");
        download_progress(app, "verifying", release.size, release.size, 0);
        verify_file(&partial, release)?;
        fs::rename(&partial, &path).map_err(error)?;
        Ok(path)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(partial);
    }
    result
}
fn verify_file(path: &Path, release: &WindowsRelease) -> Result<(), String> {
    let mut file = File::open(path).map_err(error)?;
    if file.metadata().map_err(error)?.len() != release.size {
        return Err("安装包大小校验失败".into());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut first = true;
    loop {
        let n = file.read(&mut buffer).map_err(error)?;
        if n == 0 {
            break;
        }
        if first && !buffer[..n].starts_with(b"MZ") {
            return Err("安装包不是 Windows 可执行文件".into());
        }
        first = false;
        hasher.update(&buffer[..n]);
    }
    if format!("{:x}", hasher.finalize()) != release.sha256.to_ascii_lowercase() {
        return Err("安装包 SHA-256 校验失败".into());
    }
    Ok(())
}

pub async fn execute(app: tauri::AppHandle, plan_id: String) -> Result<String, String> {
    let busy = Busy::acquire()?;
    let prepared = PLANS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(error)?
        .remove(&plan_id)
        .ok_or("操作计划不存在、已执行或应用已重启，请重新复核")?;
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = busy;
        execute_prepared(&prepared, |s| progress(&app, s))
    })
    .await
    .map_err(error)?
}
fn execute_prepared(p: &Prepared, report: impl Fn(&str)) -> Result<String, String> {
    if now() >= p.plan.expires_at {
        return Err("操作计划已过期，请重新复核".into());
    }
    p.paths.check()?;
    if fingerprint(&p.paths)? != p.fingerprint {
        return Err("安装或配置已发生变化，请重新复核".into());
    }
    if running(&p.paths.prefix) {
        return Err("企业微信仍在运行，请先从托盘退出".into());
    }
    if let (Some(path), Some(release)) = (&p.installer, &p.release) {
        check_local_path(&p.paths.home, path)?;
        verify_file(path, release)?;
    }
    let log = p.paths.root.join("operation.log");
    fs::create_dir_all(&p.paths.root).map_err(error)?;
    check_local_path(&p.paths.home, &log)?;
    match p.plan.action {
        Action::Install | Action::Update => {
            report("正在准备 Wine 环境和兼容配置…");
            apply_settings(&p.paths, &p.plan.settings, &log)?;
            report("请在企业微信官方安装向导中完成安装，保持默认安装目录。完成后请退出企业微信。");
            let mut command = wine_command(&p.paths, &p.plan.settings);
            command
                .arg(p.installer.as_ref().ok_or("安装包缺失")?)
                .arg(r"/D=C:\Program Files (x86)\WXWork");
            run(command, &log, Duration::from_secs(1800))?;
            report("等待安装向导及其子进程退出；若企业微信已自动启动，请从托盘退出。");
            wait_for_wine(&p.paths, &p.plan.settings, &log)?;
            let actual = installed_version(&p.paths.prefix);
            if actual != p.plan.target_version {
                return Err(format!(
                    "安装未完成或被取消：预期 {:?}，检测到 {:?}。请保持默认安装目录后重试。",
                    p.plan.target_version, actual
                ));
            }
            save_settings(&p.paths, &p.plan.settings)?;
            Ok(format!("企业微信 {} 安装完成", actual.unwrap_or_default()))
        }
        Action::Uninstall => {
            report("请在企业微信官方卸载向导中完成卸载。聊天记录是否保留由向导中的选项决定。");
            let path = p.paths.prefix.join(UNINSTALL);
            if !path.is_file() {
                return Err("未找到企业微信官方卸载程序，未删除任何环境数据".into());
            }
            let mut cmd = wine_command(&p.paths, &p.plan.settings);
            cmd.arg(path);
            run(cmd, &log, Duration::from_secs(1800))?;
            wait_for_wine(&p.paths, &p.plan.settings, &log)?;
            if p.paths.prefix.join(EXE).exists() {
                return Err("卸载未完成或已取消，企业微信程序仍然存在".into());
            }
            Ok("企业微信已卸载。Wine 环境目录已保留，可用于重新安装。".into())
        }
        Action::Configure => {
            report("正在应用 Wine 配置…");
            apply_settings(&p.paths, &p.plan.settings, &log)?;
            save_settings(&p.paths, &p.plan.settings)?;
            Ok("Wine 配置已应用，下次启动企业微信时生效".into())
        }
    }
}
fn wine_command(paths: &Paths, settings: &WineSettings) -> Command {
    let mut cmd = Command::new(&settings.wine_binary);
    cmd.env("WINEPREFIX", &paths.prefix)
        .env("WINEDEBUG", "-all")
        .env("LANG", "zh_CN.UTF-8");
    cmd
}
fn wait_for_wine(paths: &Paths, settings: &WineSettings, log: &Path) -> Result<(), String> {
    let runner = Path::new(&settings.wine_binary)
        .canonicalize()
        .map_err(error)?;
    let server = runner.parent().ok_or("Wine 路径无效")?.join("wineserver");
    let mut cmd = Command::new(server);
    cmd.env("WINEPREFIX", &paths.prefix).arg("-w");
    run(cmd, log, Duration::from_secs(1800))
}
fn run(mut command: Command, log: &Path, timeout: Duration) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)
        .map_err(error)?;
    command
        .stdin(Stdio::null())
        .stdout(file.try_clone().map_err(error)?)
        .stderr(file);
    let mut child = command.spawn().map_err(error)?;
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(error)? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "Wine 操作失败（{status}），日志：{}",
                    log.display()
                ))
            };
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("操作等待超时，请检查仍打开的安装向导并完成或取消后重试".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
fn reg(
    paths: &Paths,
    settings: &WineSettings,
    key: &str,
    name: &str,
    kind: &str,
    value: &str,
    log: &Path,
) -> Result<(), String> {
    let mut cmd = wine_command(paths, settings);
    cmd.args(["reg", "add", key, "/v", name, "/t", kind, "/d", value, "/f"]);
    run(cmd, log, Duration::from_secs(90))
}
// Tolerates deletion of a value that may not exist (e.g. turning a feature off
// before it was ever enabled), so apply_settings stays idempotent.
fn reg_delete_tolerate(paths: &Paths, settings: &WineSettings, key: &str, name: &str, log: &Path) {
    let mut cmd = wine_command(paths, settings);
    cmd.args(["reg", "delete", key, "/v", name, "/f"]);
    let _ = run(cmd, log, Duration::from_secs(90));
}
fn font_antialiasing_value(value: &str) -> u32 {
    match value {
        "gray" => 1,
        "rgb" => 2,
        "bgr" => 3,
        _ => 0, // default
    }
}
fn font_hinting_value(value: &str) -> u32 {
    match value {
        "none" => 1,
        "light" => 2,
        "medium" => 3,
        "full" => 4,
        _ => 0, // default
    }
}
fn apply_settings(paths: &Paths, settings: &WineSettings, log: &Path) -> Result<(), String> {
    settings.validate()?;
    if !font_available() {
        return Err("缺少 fonts-noto-cjk 中文字体".into());
    }
    // Only compatibility keys are written; no copying of user.reg / system.reg,
    // account IDs, chat history, hardware GUIDs, or user-specific font paths.
    fs::create_dir_all(&paths.prefix).map_err(error)?;
    paths.check()?;
    let mut boot = wine_command(paths, settings);
    boot.args(["wineboot", "--init"]);
    run(boot, log, Duration::from_secs(180))?;
    let mut version = wine_command(paths, settings);
    version.args(["winecfg", "-v", &settings.windows_version]);
    run(version, log, Duration::from_secs(90))?;
    reg(
        paths,
        settings,
        r"HKCU\Control Panel\Desktop",
        "LogPixels",
        "REG_DWORD",
        &settings.dpi.to_string(),
        log,
    )?;
    reg(
        paths,
        settings,
        r"HKCU\Software\Wine\Fonts",
        "LogPixels",
        "REG_DWORD",
        &settings.dpi.to_string(),
        log,
    )?;
    reg(
        paths,
        settings,
        r"HKCU\Software\Wine\Drivers",
        "Graphics",
        "REG_SZ",
        &settings.graphics_driver,
        log,
    )?;
    for name in ["Tahoma", "Tahoma Bold"] {
        reg(
            paths,
            settings,
            r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\FontSubstitutes",
            name,
            "REG_SZ",
            &settings.chinese_font,
            log,
        )?;
    }
    // Font smoothing (antialiasing + hinting) for CJK text legibility.
    reg(
        paths,
        settings,
        r"HKCU\Software\Wine\Fonts",
        "Antialiasing",
        "REG_DWORD",
        &font_antialiasing_value(&settings.font_antialiasing).to_string(),
        log,
    )?;
    reg(
        paths,
        settings,
        r"HKCU\Software\Wine\Fonts",
        "HintStyle",
        "REG_DWORD",
        &font_hinting_value(&settings.font_hinting).to_string(),
        log,
    )?;
    // FontLink: route Latin UI faces missing CJK glyphs to the installed Noto CJK
    // bundle so Chinese renders instead of empty boxes.
    const FONT_LINK_KEY: &str = r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink";
    const FONT_LINK_VALUE: &str = "NotoSansCJK-Regular.ttc,Noto Sans CJK SC";
    for name in ["Tahoma", "Arial", "MS Sans Serif", "MS Shell Dlg"] {
        if settings.font_link {
            reg(paths, settings, FONT_LINK_KEY, name, "REG_MULTI_SZ", FONT_LINK_VALUE, log)?;
        } else {
            reg_delete_tolerate(paths, settings, FONT_LINK_KEY, name, log);
        }
    }
    // Virtual desktop (resolution x color depth). "off" clears the value to
    // restore the real X desktop instead of a synthetic one.
    if settings.virtual_desktop == "off" {
        reg_delete_tolerate(
            paths,
            settings,
            r"HKCU\Software\Wine\Explorer\Desktops",
            "Default",
            log,
        );
    } else {
        let desktop = format!("{}x{}", settings.virtual_desktop, settings.color_depth);
        reg(
            paths,
            settings,
            r"HKCU\Software\Wine\Explorer\Desktops",
            "Default",
            "REG_SZ",
            &desktop,
            log,
        )?;
    }
    write_helper(paths)?;
    Ok(())
}
fn save_settings(paths: &Paths, settings: &WineSettings) -> Result<(), String> {
    fs::create_dir_all(&paths.root).map_err(error)?;
    let path = paths.root.join("settings.json");
    check_local_path(&paths.home, &path)?;
    let temp = paths.root.join("settings.tmp");
    check_local_path(&paths.home, &temp)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .map_err(error)?;
    file.write_all(&serde_json::to_vec_pretty(settings).map_err(error)?)
        .map_err(error)?;
    file.sync_all().map_err(error)?;
    fs::rename(temp, path).map_err(error)
}
fn write_helper(paths: &Paths) -> Result<PathBuf, String> {
    fs::create_dir_all(&paths.root).map_err(error)?;
    let path = paths.root.join("wecom-titlebar.exe");
    check_local_path(&paths.home, &path)?;
    // Recreate from compiled-in bytes; never trust a downloaded compatibility script.
    if fs::read(&path).ok().as_deref() != Some(HELPER) {
        fs::write(&path, HELPER).map_err(error)?;
    }
    Ok(path)
}
fn spawn_detached(command: &mut Command) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(error)?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
pub async fn launch() -> Result<(), String> {
    let busy = Busy::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = busy;
        let paths = Paths::new()?;
        let settings = paths.settings()?;
        if !paths.prefix.join(EXE).is_file() {
            return Err("企业微信尚未安装".into());
        }
        if settings.titlebar_fix {
            let helper = write_helper(&paths)?;
            spawn_detached(wine_command(&paths, &settings).arg(helper))?;
        }
        spawn_detached(wine_command(&paths, &settings).arg(paths.prefix.join(EXE)))?;
        Ok(())
    })
    .await
    .map_err(error)?
}

#[tauri::command]
pub async fn get_windows_state() -> Result<WindowsState, String> {
    state().await
}
#[tauri::command]
pub async fn prepare_windows_operation(
    app: tauri::AppHandle,
    action: Action,
    settings: WineSettings,
) -> Result<Plan, String> {
    prepare(app, action, settings).await
}
#[tauri::command]
pub async fn execute_windows_operation(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<String, String> {
    execute(app, plan_id).await
}
#[tauri::command]
pub async fn launch_windows_application() -> Result<(), String> {
    launch().await
}
#[tauri::command]
pub async fn open_windows_directory(app: tauri::AppHandle) -> Result<(), String> {
    let paths = Paths::new()?;
    let path = if paths.prefix.is_dir() {
        paths.prefix
    } else {
        app.path().app_data_dir().map_err(error)?
    };
    spawn_detached(Command::new("/usr/bin/gio").arg("open").arg(path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "umanager-windows-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn release_fixture() -> WindowsRelease {
        WindowsRelease {
            display_name: "企业微信".into(),
            profile: "wecom-v1".into(),
            version: "5.0.10.6015".into(),
            download_url: "https://dldir1.qq.com/wework/work_weixin/WeCom_5.0.10.6015.exe".into(),
            download_hosts: vec!["dldir1.qq.com".into()],
            size: 2048,
            sha256: "a".repeat(64),
        }
    }
    #[test]
    fn release_rejects_unsafe_sources() {
        let good = release_fixture();
        assert!(good.validate().is_ok());
        for url in [
            "http://dldir1.qq.com/x",
            "https://dldir1.qq.com.evil.test/x",
            "https://user@dldir1.qq.com/x",
            "https://dldir1.qq.com:8443/x",
            "https://dldir1.qq.com/latest.exe",
        ] {
            let mut r = good.clone();
            r.download_url = url.into();
            assert!(r.validate().is_err());
        }
        let mut r = good.clone();
        r.download_hosts.push("*.qq.com".into());
        assert!(r.validate().is_err());
        let mut r = good;
        r.version = "5.0.10.6014".into();
        assert!(r.validate().is_err());
    }
    #[test]
    fn numeric_versions_do_not_sort_lexicographically() {
        assert!(version_parts("5.0.10.1") > version_parts("5.0.9.9999"));
        assert!(version_parts("5.0.1").is_none());
        assert!(version_parts("5.0.1.-1").is_none());
    }
    #[test]
    fn reads_executable_version_not_leftover_directories() {
        let f = Fixture::new();
        let paths = Paths::at(f.0.clone()).unwrap();
        fs::create_dir_all(paths.prefix.join(EXE).parent().unwrap()).unwrap();
        let mut bytes = b"MZpadding".to_vec();
        bytes.extend([0xbd, 4, 0xef, 0xfe, 0, 0, 1, 0]);
        bytes.extend((5u32 << 16).to_le_bytes());
        bytes.extend(((10u32 << 16) | 6015).to_le_bytes());
        fs::write(paths.prefix.join(EXE), bytes).unwrap();
        assert_eq!(
            installed_version(&paths.prefix).as_deref(),
            Some("5.0.10.6015")
        );
        fs::remove_file(paths.prefix.join(EXE)).unwrap();
        fs::create_dir_all(
            paths
                .prefix
                .join("drive_c/Program Files (x86)/WXWork/5.0.10.6015"),
        )
        .unwrap();
        assert!(installed_version(&paths.prefix).is_none());
    }
    #[test]
    fn existing_prefix_is_adopted_without_modifying_it() {
        let f = Fixture::new();
        let prefix = f.0.join(".local/share/wineprefixes/wecom");
        fs::create_dir_all(&prefix).unwrap();
        fs::write(prefix.join("sentinel"), "unchanged").unwrap();
        let p = Paths::at(f.0.clone()).unwrap();
        assert_eq!(p.prefix, prefix);
        assert_eq!(
            fs::read_to_string(prefix.join("sentinel")).unwrap(),
            "unchanged"
        );
        assert!(!p.root.exists());
    }
    #[test]
    fn rejects_symlinked_prefix_and_program() {
        let f = Fixture::new();
        let outside = Fixture::new();
        let base = f.0.join(".local/share/wineprefixes");
        fs::create_dir_all(&base).unwrap();
        symlink(&outside.0, base.join("wecom")).unwrap();
        assert!(Paths::at(f.0.clone()).is_err());
        fs::remove_file(base.join("wecom")).unwrap();
        let paths = Paths::at(f.0.clone()).unwrap();
        fs::create_dir_all(paths.prefix.join(EXE).parent().unwrap()).unwrap();
        symlink("/bin/true", paths.prefix.join(EXE)).unwrap();
        assert!(paths.check().is_err());
        assert!(check_local_path(&f.0, &f.0.join("../escape")).is_err());
    }
    #[test]
    fn changed_or_expired_plan_cannot_execute() {
        let f = Fixture::new();
        let paths = Paths::at(f.0.clone()).unwrap();
        let mut p = Prepared {
            plan: Plan {
                plan_id: "test".into(),
                action: Action::Uninstall,
                prefix: paths.prefix.display().to_string(),
                installed_version: None,
                target_version: None,
                settings: WineSettings::default(),
                expires_at: now() - 1,
                download_size: None,
                sha256: None,
            },
            fingerprint: fingerprint(&paths).unwrap(),
            paths,
            installer: None,
            release: None,
        };
        assert!(execute_prepared(&p, |_| {}).unwrap_err().contains("过期"));
        p.plan.expires_at = now() + 900;
        fs::create_dir_all(p.paths.prefix.join(EXE).parent().unwrap()).unwrap();
        fs::write(p.paths.prefix.join(EXE), "changed").unwrap();
        assert!(execute_prepared(&p, |_| {}).unwrap_err().contains("变化"));
    }
    #[test]
    fn altered_installer_is_rejected_before_execution() {
        let f = Fixture::new();
        let file = f.0.join("installer.exe");
        let mut bytes = vec![0u8; 2048];
        bytes[..2].copy_from_slice(b"MZ");
        fs::write(&file, &bytes).unwrap();
        let mut r = release_fixture();
        r.sha256 = format!("{:x}", Sha256::digest(&bytes));
        assert!(verify_file(&file, &r).is_ok());
        bytes[10] = 1;
        fs::write(&file, &bytes).unwrap();
        assert!(verify_file(&file, &r).is_err());
        fs::write(&file, "MZ").unwrap();
        assert!(verify_file(&file, &r).is_err());
    }
    #[test]
    fn configuration_cannot_inject_commands_or_arbitrary_paths() {
        let mut settings = WineSettings::default();
        assert!(settings.validate().is_ok());
        settings.wine_binary = "/tmp/evil".into();
        assert!(settings.validate().is_err());
        settings = WineSettings::default();
        settings.windows_version = "win10; rm -rf x".into();
        assert!(settings.validate().is_err());
        settings = WineSettings::default();
        settings.dpi = 0;
        assert!(settings.validate().is_err());
    }
    /// Run under Xvfb with WINEDLLOVERRIDES=winemenubuilder.exe=d. Uses a
    /// disposable prefix and synthetic PE installers, never the user's WeCom.
    #[test]
    #[ignore = "requires Wine, fonts-noto-cjk, MinGW and an isolated display"]
    fn wine_install_update_configure_uninstall_lifecycle() {
        let f = Fixture::new();
        let paths = Paths::at(f.0.clone()).unwrap();
        let compiler =
            std::env::var("UMANAGER_MINGW_CC").unwrap_or("x86_64-w64-mingw32-gcc".into());
        for (patch, action) in [(1, Action::Install), (2, Action::Update)] {
            let installer = f.0.join(format!("fixture-{patch}.exe"));
            let status = Command::new(&compiler)
                .args(["-Os", "-static"])
                .arg(format!("-DPATCH_VERSION={patch}"))
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../tests/fixtures/windows-installer.c"
                ))
                .arg("-o")
                .arg(&installer)
                .status()
                .unwrap();
            assert!(status.success());
            let bytes = fs::read(&installer).unwrap();
            let mut r = release_fixture();
            r.version = format!("5.0.{patch}.0");
            r.size = bytes.len() as u64;
            r.sha256 = format!("{:x}", Sha256::digest(&bytes));
            let p = Prepared {
                plan: Plan {
                    plan_id: "fixture".into(),
                    action,
                    prefix: paths.prefix.display().to_string(),
                    installed_version: installed_version(&paths.prefix),
                    target_version: Some(r.version.clone()),
                    settings: WineSettings::default(),
                    expires_at: now() + 900,
                    download_size: Some(r.size),
                    sha256: Some(r.sha256.clone()),
                },
                paths: paths.clone(),
                fingerprint: fingerprint(&paths).unwrap(),
                installer: Some(installer),
                release: Some(r),
            };
            execute_prepared(&p, |s| eprintln!("{s}")).unwrap();
            assert_eq!(
                installed_version(&paths.prefix),
                Some(format!("5.0.{patch}.0"))
            );
        }
        for action in [Action::Configure, Action::Uninstall] {
            let mut settings = WineSettings::default();
            settings.dpi = 144;
            let p = Prepared {
                plan: Plan {
                    plan_id: "fixture".into(),
                    action: action.clone(),
                    prefix: paths.prefix.display().to_string(),
                    installed_version: installed_version(&paths.prefix),
                    target_version: None,
                    settings,
                    expires_at: now() + 900,
                    download_size: None,
                    sha256: None,
                },
                paths: paths.clone(),
                fingerprint: fingerprint(&paths).unwrap(),
                installer: None,
                release: None,
            };
            execute_prepared(&p, |s| eprintln!("{s}")).unwrap();
            if action == Action::Configure {
                assert_eq!(paths.settings().unwrap().dpi, 144);
            }
        }
        assert!(!paths.prefix.join(EXE).exists());
        assert!(
            paths.prefix.join("user.reg").exists(),
            "uninstall preserves the prefix"
        );
    }
}
