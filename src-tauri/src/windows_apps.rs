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
/// 企业微信只有简体中文版：安装向导、界面和帮助都是中文，Windows 侧也必须按中文区域
/// 运行（ANSI 代码页 936/GBK），否则界面变英文、GBK 文本变成乱码。App 自身可能继承
/// 英文或 C locale（从终端启动、部分 .desktop 启动器、英文系统默认），而 glibc 里
/// `LC_ALL` 优先于 `LANG`：只设置 `LANG=zh_CN.UTF-8` 时继承来的 `LC_ALL=C.UTF-8`
/// 仍然生效，Wine 于是按英文区域启动。所以每个 Wine 进程都显式覆盖下面四个变量。
const CHINESE_LOCALE: &str = "zh_CN.UTF-8";
const CHINESE_LANGUAGE: &str = "zh_CN:zh";
/// 启动后观察窗口：足够 Wine 客户端连上服务端并报出版本冲突，又不至于让窗口显形变慢。
const LAUNCH_GRACE: Duration = Duration::from_secs(8);
/// 启动进程立即退出后，再等这么久确认真正的进程有没有接班（企业微信的引导进程会先退出）。
const POST_EXIT_CONFIRM_POLLS: u32 = 30;
/// `wineserver` 客户端在协议版本不一致时打印这个字符串后 abort
/// （dlls/ntdll/unix/server.c: server_protocol_error("version mismatch …")）。
const VERSION_MISMATCH: &str = "version mismatch";
/// Wine 的 GDI 只读**机器**里的 SystemLink：`load_system_links()`
/// （dlls/win32u/font.c，wine-11.20 第 105-115 行）打开的是
/// `\Registry\Machine\Software\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink`。
/// 也就是说 HKLM 才是被读取的路径（Windows 自己用 HKCU，Wine 不是）。
const FONT_LINK_KEY: &str =
    r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink";
/// 旧版本写错的 HKCU 同名路径：Wine 从不会读它，配置时顺手删掉，避免留下
/// 「看起来已经配置过」的死数据。
const LEGACY_FONT_LINK_KEY: &str =
    r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink";
/// `文件名,字族名`，与 Wine 内置 SystemLink 条目同一格式（如 `msyh.ttc,Microsoft YaHei`）。
/// 文件名必须是 Wine 字体枚举能看到的宿主字体：Ubuntu 的 fonts-noto-cjk 提供
/// `NotoSansCJK-Regular.ttc`，族名 `Noto Sans CJK SC`。
const FONT_LINK_VALUE: &str = "NotoSansCJK-Regular.ttc,Noto Sans CJK SC";
/// 要链接到中文字体的字族：只写**宿主上不存在**的那些（Windows 中文界面常用的微软雅黑、
/// 宋体，以及 Windows 专有的 Microsoft Sans Serif）。
///
/// 实测（wine-11.20 + fonts-noto-cjk，一次性 prefix）：
/// - 字族在宿主上不存在时，Wine 才会沿 SystemLink 选到 Noto Sans CJK SC：英文区域下
///   Microsoft YaHei / Microsoft YaHei UI / SimSun / NSimSun 从 Arial/0xFFFF 变成
///   Noto Sans CJK SC/0x2548、0x4E44；把整个 SystemLink 键删掉又变回 0xFFFF。
/// - 给 Tahoma / Arial / MS Sans Serif 这类宿主上**存在**的字族写 SystemLink 会顶掉
///   Wine 自己的 fontconfig 回退，把 GB2312 场景从「有中文字形」变成 0xFFFF ——
///   所以这里绝不能写它们，反而要按 `FONT_LINK_HARMFUL_FACES` 清理。
/// - `Microsoft Sans Serif` 在某些 prefix 里已被 wineboot 写进 FontSubstitutes，而 Wine 会
///   忽略「被替换字体」的 SystemLink 条目（font.c: `get_gdi_font_subst` 命中即跳过），
///   那种情况下这条不起作用，留着也无害。
const FONT_LINK_FACES: [&str; 5] = [
    "Microsoft Sans Serif",
    "Microsoft YaHei",
    "Microsoft YaHei UI",
    "SimSun",
    "NSimSun",
];
/// 不能写 SystemLink 的宿主既有字族（写了会让中文变差，见上）。这几个名字不在 wineboot
/// 写入的默认条目里，所以删除 HKLM 里的同名条目是安全的（只可能是用户手写的）。
const FONT_LINK_HARMFUL_FACES: [&str; 3] = ["Arial", "MS Sans Serif", "MS Shell Dlg"];
/// 旧版本把 SystemLink 写在这四个名字的 HKCU 路径下：Wine 从不读 HKCU，等于死数据，
/// 配置时清掉，避免留下「看起来已经配置过」的假象。只清 HKCU —— HKLM 里的 `Tahoma`
/// 是 wineboot 自带并会重新生成的默认条目，不属于我们要删的东西。
const LEGACY_FONT_LINK_FACES: [&str; 4] =
    ["Tahoma", "Arial", "MS Sans Serif", "MS Shell Dlg"];
/// 字体替换（HKLM FontSubstitutes，同样只有 HKLM 会被 GDI 读取）。沿用原有行为：
/// Tahoma / Tahoma Bold 整体替换成中文字体。实测说明这个替换只在「请求的字族缺失，或
/// 字符集和该字族不匹配」时生效，救不了 Tahoma / Arial 在 DEFAULT_CHARSET 下的中文，
/// 因此它是补充手段，不能替代下面的本地化修复。
const FONT_SUBSTITUTE_FACES: [&str; 2] = ["Tahoma", "Tahoma Bold"];
const FONT_SUBSTITUTES_KEY: &str =
    r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\FontSubstitutes";
/// 中文区域的 Windows 表示。Wine 自己的 `update_locale_registry()`
/// （dlls/kernelbase/locale.c）就是按这三个值写 `HKCU\Control Panel\International` 的：
/// `Locale` 是 `%08x` 的 GetUserDefaultLCID（zh-CN = 0x0804），`sLanguage` 是
/// LOCALE_SABBREVLANGNAME（CHS），`LocaleName` 是 LOCALE_SNAME（zh-CN）。
const CHINESE_REGION: [(&str, &str); 3] = [
    ("Locale", "00000804"),
    ("sLanguage", "CHS"),
    ("LocaleName", "zh-CN"),
];
const INTERNATIONAL_KEY: &str = r"HKCU\Control Panel\International";
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
    /// 该版本安装包的发布时间（与主程序 feed 条目同一套语义），只用于展示与
    /// 商店列表排序；签名源没有该信息时为 `None`。
    #[serde(default)]
    pub version_updated_at_unix_seconds: Option<u64>,
    #[serde(default)]
    pub version_updated_at_source: Option<crate::feed::VersionUpdatedAtSource>,
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
        crate::feed::validate_version_updated_at(
            &self.version_updated_at_unix_seconds,
            &self.version_updated_at_source,
        )?;
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
    /// 该 Wine 环境里是否还有进程（wineserver / 辅助进程 / 企业微信）。升级 Wine 后
    /// 残留的旧 wineserver 会让每次启动都失败，而企业微信主程序并没有在运行，界面
    /// 需要据此仍然给出「清理残留会话」入口。
    session_active: bool,
    /// 宿主是否真的生成了 zh_CN.UTF-8。缺失时中文本地化无法强制注入，界面要提示
    /// 安装中文语言包。
    chinese_locale_available: bool,
    font_available: bool,
    feed_error: Option<String>,
    busy: bool,
    /// 候选安装包的发布时间（签名 feed 的 Windows 条目），用于商店列表排序。
    version_updated_at_unix_seconds: Option<u64>,
    version_updated_at_source: Option<crate::feed::VersionUpdatedAtSource>,
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
        let processes = prefix_processes(&paths.prefix);
        let version_updated_at = release
            .as_ref()
            .map(|r| (r.version_updated_at_unix_seconds, r.version_updated_at_source))
            .unwrap_or((None, None));
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
            running: processes.iter().any(is_wecom_process),
            session_active: !processes.is_empty(),
            chinese_locale_available: chinese_locale_available(),
            font_available: font_available(),
            settings,
            feed_error,
            busy: BUSY.load(Ordering::SeqCst),
            version_updated_at_unix_seconds: version_updated_at.0,
            version_updated_at_source: version_updated_at.1,
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
/// 该 Wine 环境里的一个进程。`argv` 是归一化后的**完整**命令行（每个参数去引号、
/// 反斜杠转 `/`、小写）：Wine 启动 Windows 程序时命令行可能是
/// `wine /prefix/drive_c/.../WXWork.exe`（程序路径在 argv[1]），也可能第一个参数就是
/// 程序，只看 argv[0] 会漏判正在运行的企业微信。
#[derive(Clone, Debug, PartialEq, Eq)]
struct SessionProcess {
    pid: i32,
    argv: Vec<String>,
}
/// 只有 Wine 自己的进程才算这个环境的会话成员。用户可能在 shell 里
/// `export WINEPREFIX=…` 之后再启动 UManager，这个环境变量于是也会出现在终端、甚至
/// UManager 自己身上；只看环境变量会把它们当成残留会话，`stop()` 时还会杀掉自己。
/// 主判据是 `/proc/<pid>/exe` 的文件名含 `wine`：实测 wine-11.20 里所有会话进程
/// （wineserver、wine/wine-preloader 加载器、它启动的每个 Windows 程序）都是这样。
/// 只有连 exe 链接都读不到时才退回「命令行里有 `*.exe`」这一较弱判据 —— 单看 argv 会把
/// `tail -f x.exe`、`unzip WeCom.exe` 这类普通命令也算成会话成员。
fn is_wine_process(pid: i32, argv: &[String]) -> bool {
    if pid == std::process::id() as i32 {
        return false;
    }
    match fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(exe) => exe
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .is_some_and(|name| name.contains("wine")),
        Err(_) => argv.iter().any(|arg| arg.ends_with(".exe")),
    }
}
/// 环境里的 `WINEPREFIX` 与目标 prefix 是否是同一个目录。Wine 不做路径规范化：实测
/// `WINEPREFIX=/pfx/` 在 environ 里就带尾斜杠，用户也可能用符号链接路径；所以先按去掉
/// 尾斜杠的字符串比较，再退回 `canonicalize`。
fn same_prefix(value: &str, prefix: &Path) -> bool {
    let target = prefix.display().to_string();
    if value.trim_end_matches('/') == target.trim_end_matches('/') {
        return true;
    }
    match (fs::canonicalize(value), fs::canonicalize(prefix)) {
        (Ok(value), Ok(target)) => value == target,
        _ => false,
    }
}
fn environ_matches_prefix(environ: &Path, prefix: &Path) -> bool {
    fs::read(environ)
        .unwrap_or_default()
        .split(|byte| *byte == 0)
        .filter_map(|value| std::str::from_utf8(value).ok())
        .filter_map(|value| value.strip_prefix("WINEPREFIX="))
        .any(|value| same_prefix(value, prefix))
}
/// 列出属于这个 prefix 的所有进程。Wine 会把 `WINEPREFIX` 传给 wineserver 以及它启动
/// 的每个辅助进程，所以环境变量是「这个环境」精确的标记；`/proc/<pid>/environ` 读不到
/// （其它用户）时视为不属于本环境。
fn prefix_processes(prefix: &Path) -> Vec<SessionProcess> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let argv: Vec<String> = fs::read(entry.path().join("cmdline"))
            .unwrap_or_default()
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| {
                String::from_utf8_lossy(arg)
                    .trim_matches('"')
                    .replace('\\', "/")
                    .to_ascii_lowercase()
            })
            .collect();
        if argv.is_empty()
            || !environ_matches_prefix(&entry.path().join("environ"), prefix)
            || !is_wine_process(pid, &argv)
        {
            continue;
        }
        found.push(SessionProcess { pid, argv });
    }
    found
}
/// 任意一个参数指向 WXWork.exe 就算（与旧实现同一语义）。
fn is_wecom_process(process: &SessionProcess) -> bool {
    process
        .argv
        .iter()
        .any(|arg| arg.ends_with("/wxwork.exe") || arg == "wxwork.exe")
}
/// 企业微信主程序是否在运行。残留的 wineserver / 辅助进程不算「企业微信在运行」：
/// 它们不该出现在「请先退出企业微信」这类判断里，但必须能被 `stop()` 清掉。
fn running(prefix: &Path) -> bool {
    prefix_processes(prefix).iter().any(is_wecom_process)
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
    // 安装/更新在本阶段只下载并校验安装包（可能数分钟），用户完全可以继续使用企业
    // 微信，运行状态留到 execute 的执行前再把关；卸载与应用配置不下载，仍在复核前就
    // 拦下来，避免弹出一个注定失败的确认框。
    if matches!(action, Action::Uninstall | Action::Configure) && running(&paths.prefix) {
        return Err("请先从托盘退出企业微信，再进行卸载或应用配置".into());
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
    if action != Action::Uninstall {
        require_chinese_locale()?;
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
            report("安装向导已退出，正在确认安装结果…");
            let mut probe = VersionProbe::new(&p.paths.prefix, p.plan.target_version.as_deref());
            wait_for_wine(&p.paths, &p.plan.settings, &log, || probe.landed())?;
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
            // The uninstaller is done as soon as the program is gone — the same
            // criterion checked below — so a lingering Wine session must not keep
            // the dialog waiting.
            let exe = p.paths.prefix.join(EXE);
            wait_for_wine(&p.paths, &p.plan.settings, &log, || !exe.exists())?;
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
/// 宿主是否已经生成 zh_CN.UTF-8。强制一个 glibc 不认识的 locale 会让 `setlocale`
/// 失败并静默退回 C 区域，比保留继承值更糟，所以只有确认存在时才覆盖。只缓存**肯定**
/// 的结论：用户照着提示装好语言包后，不重启 UManager 也能直接重试成功。
fn chinese_locale_available() -> bool {
    static AVAILABLE: OnceLock<()> = OnceLock::new();
    if AVAILABLE.get().is_some() {
        return true;
    }
    let detected = match Command::new("/usr/bin/locale").arg("-a").output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line.trim().to_ascii_lowercase().replace('-', "") == "zh_cn.utf8"),
        // `locale` 本身跑不起来时不能误报「缺少中文语言包」：退回当前会话 locale。
        _ => ["LC_ALL", "LC_MESSAGES", "LANG"].iter().any(|key| {
            std::env::var(key).is_ok_and(|value| value.to_ascii_lowercase().starts_with("zh_cn"))
        }),
    };
    if detected {
        let _ = AVAILABLE.set(());
    }
    detected
}
/// 缺少中文 locale 时的统一提示：不满足这个条件，企业微信就是英文界面，
/// 所以安装、配置和启动都直接拦下来，并给出可执行的修复方式。
fn require_chinese_locale() -> Result<(), String> {
    if chinese_locale_available() {
        return Ok(());
    }
    Err(
        "系统缺少 zh_CN.UTF-8 中文区域设置：企业微信会以英文界面启动、中文可能显示异常。\
         请先安装中文语言包（sudo apt install language-pack-zh-hans），或生成该区域\
         （sudo locale-gen zh_CN.UTF-8）后重试"
            .into(),
    )
}
/// 把中文区域显式覆盖到 Wine 进程上。`LC_ALL` 必须一起设置：它优先于 `LANG`，只设
/// `LANG` 会被 App 继承来的 `LC_ALL=C.UTF-8` 盖掉；`LANGUAGE` 决定 gettext 取哪份
/// 翻译（Wine 自己的 MUI/提示），`LC_MESSAGES` 兜底。
fn apply_chinese_locale(command: &mut Command) -> &mut Command {
    command
        .env("LANG", CHINESE_LOCALE)
        .env("LC_ALL", CHINESE_LOCALE)
        .env("LC_MESSAGES", CHINESE_LOCALE)
        .env("LANGUAGE", CHINESE_LANGUAGE)
}
fn wine_command(paths: &Paths, settings: &WineSettings) -> Command {
    let mut cmd = Command::new(&settings.wine_binary);
    cmd.env("WINEPREFIX", &paths.prefix)
        .env("WINEDEBUG", "-all");
    if chinese_locale_available() {
        apply_chinese_locale(&mut cmd);
    }
    cmd
}
fn wineserver_path(settings: &WineSettings) -> Result<PathBuf, String> {
    let runner = Path::new(&settings.wine_binary)
        .canonicalize()
        .map_err(error)?;
    Ok(runner.parent().ok_or("Wine 路径无效")?.join("wineserver"))
}
/// `installed_version`, but only re-parsing the executable when it actually
/// changed. The WeCom main binary is ~257 MB, so a plain 500 ms poll would
/// re-read (and re-allocate) a quarter of a gigabyte every time. Starts without a
/// stamp so the first poll always checks, even when the installer had already
/// replaced the file before the wait started.
struct VersionProbe<'a> {
    prefix: &'a Path,
    target: Option<&'a str>,
    stamp: Option<(u64, Option<SystemTime>)>,
    landed: bool,
}
impl<'a> VersionProbe<'a> {
    fn new(prefix: &'a Path, target: Option<&'a str>) -> Self {
        Self {
            prefix,
            target,
            stamp: None,
            landed: false,
        }
    }
    /// Whether the target version is on disk now. Latches: once the installer has
    /// been observed writing the target build, the wait is over even though the
    /// file then stops changing.
    fn landed(&mut self) -> bool {
        if self.landed {
            return true;
        }
        let current = fs::metadata(self.prefix.join(EXE))
            .ok()
            .map(|meta| (meta.len(), meta.modified().ok()));
        if current == self.stamp {
            return false;
        }
        self.stamp = current;
        self.landed =
            self.target.is_some() && installed_version(self.prefix).as_deref() == self.target;
        self.landed
    }
}
/// Wait for the Wine session to wind down, but stop as soon as `landed` reports
/// that the operation has already completed.
///
/// `wineserver -w` returns only once *every* process in the prefix is gone. The
/// WeCom installer auto-starts 企业微信, whose Wine/CEF processes outlive the
/// installer, so waiting for the whole session left the dialog on 「执行中…」 for
/// the full 30-minute timeout after a finished installation — the installer had
/// already exited and the version check right after this would have passed.
/// The Wine server is only a *waiter* here (it is a client of `wineserver`), so
/// ending the wait early — or on timeout — kills that waiter, never the prefix.
fn wait_for_wine(
    paths: &Paths,
    settings: &WineSettings,
    log: &Path,
    landed: impl FnMut() -> bool,
) -> Result<(), String> {
    let mut cmd = Command::new(wineserver_path(settings)?);
    cmd.env("WINEPREFIX", &paths.prefix).arg("-w");
    if chinese_locale_available() {
        apply_chinese_locale(&mut cmd);
    }
    let mut waiter = spawn_logged(cmd, log)?;
    wait_for_settle(&mut waiter, Duration::from_secs(1800), landed)
}
/// Poll until the waiter process exits, `landed` becomes true, or `timeout`
/// elapses. Either way the waiter is killed and reaped, so no zombie is left.
fn wait_for_settle(
    waiter: &mut std::process::Child,
    timeout: Duration,
    mut landed: impl FnMut() -> bool,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        if waiter.try_wait().map_err(error)?.is_some() {
            return Ok(());
        }
        if landed() {
            let _ = waiter.kill();
            let _ = waiter.wait();
            return Ok(());
        }
        if start.elapsed() > timeout {
            let _ = waiter.kill();
            let _ = waiter.wait();
            return Err("操作等待超时，请检查仍打开的安装向导并完成或取消后重试".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
/// Spawn a Wine command with its output appended to the operation log.
fn spawn_logged(mut command: Command, log: &Path) -> Result<std::process::Child, String> {
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
    command.spawn().map_err(error)
}
fn run(command: Command, log: &Path, timeout: Duration) -> Result<(), String> {
    let mut child = spawn_logged(command, log)?;
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
    for name in FONT_SUBSTITUTE_FACES {
        reg(
            paths,
            settings,
            FONT_SUBSTITUTES_KEY,
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
    // bundle so Chinese renders instead of empty boxes. 必须写 HKLM：Wine 的
    // `load_system_links()` 只打开 \Registry\Machine\…\FontLink\SystemLink，旧版本写
    // 的 HKCU 路径从来没被读过，所以中文一直是方框。
    for name in FONT_LINK_FACES {
        if settings.font_link {
            reg(
                paths,
                settings,
                FONT_LINK_KEY,
                name,
                "REG_MULTI_SZ",
                FONT_LINK_VALUE,
                log,
            )?;
        } else {
            reg_delete_tolerate(paths, settings, FONT_LINK_KEY, name, log);
        }
        reg_delete_tolerate(paths, settings, LEGACY_FONT_LINK_KEY, name, log);
    }
    // 宿主既有字族的 SystemLink 条目会把中文渲染从「fontconfig 回退」变成「缺字形」，
    // 见 FONT_LINK_HARMFUL_FACES 的注释。
    for name in FONT_LINK_HARMFUL_FACES {
        reg_delete_tolerate(paths, settings, FONT_LINK_KEY, name, log);
    }
    // 旧版本写在 HKCU 的那份是死数据，全部清掉。
    for name in LEGACY_FONT_LINK_FACES {
        reg_delete_tolerate(paths, settings, LEGACY_FONT_LINK_KEY, name, log);
    }
    // 中文区域与界面语言修复。实测结论（wine-11.20，固定注册表只改环境）：真正决定
    // GetUserDefaultLCID / GetUserDefaultUILanguage / GetACP 的是进程 locale
    // （dlls/ntdll/unix/env.c: setlocale(LC_ALL, "") 取 LC_CTYPE/LC_MESSAGES；
    // LC_ALL 压过 LANG），注册表这三个值单独改**不影响** LCID —— 启动时 Wine 反而会用
    // 环境 locale 把注册表改回去（+nls trace: "locale changed L\"zh-CN\" -> L\"en-US\""）。
    // 所以：环境那份由 `apply_chinese_locale` 在每次启动时保证（主修复）；这里写的是
    // Wine 自己也会按环境重写的那份镜像，让直接读 HKCU\Control Panel\International 的
    // 应用与 Windows 行为一致（次要修复）。
    for (name, value) in CHINESE_REGION {
        reg(
            paths,
            settings,
            INTERNATIONAL_KEY,
            name,
            "REG_SZ",
            value,
            log,
        )?;
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
/// 启动类日志：每次启动截断重建，`launch_checked` 在进程立即退出时靠它判断原因。
/// 主程序与标题栏助手分开写，避免长期运行的助手把噪声混进主程序的日志。
fn prepare_log(paths: &Paths, name: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(&paths.root).map_err(error)?;
    let log = paths.root.join(name);
    check_local_path(&paths.home, &log)?;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&log)
        .map_err(error)?;
    Ok(log)
}
/// 立即退出时，日志最后一行通常就是真正的原因（Wine 的报错、缺 DLL、版本冲突）。
fn last_log_line(log: &str) -> String {
    log.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .unwrap_or_default()
        .to_owned()
}
/// Wine 只在客户端与服务端协议版本一致时才运行程序：升级 Wine 后旧的 wineserver
/// 仍占着这个 prefix 的 socket，新客户端会打印 `version mismatch` 并中止
/// （dlls/ntdll/unix/server.c）。这属于「清理残留会话后重试」，不是重装。
fn version_mismatch(log: &str) -> bool {
    log.to_ascii_lowercase().contains(VERSION_MISMATCH)
}
/// 启动进程立即退出时的用户可见原因。
fn launch_failure(prefix: &Path, log: &Path) -> String {
    let text = fs::read_to_string(log).unwrap_or_default();
    if version_mismatch(&text) {
        return format!(
            "启动失败：Wine 版本不一致（环境：{}）。升级 Wine 后旧的 wineserver 仍在这个环境里运行，\
             新的 Wine 客户端无法与它通信。请点「强制停止」清理该环境的残留进程后重新打开。日志：{}",
            prefix.display(),
            log.display()
        );
    }
    let detail = last_log_line(&text);
    if detail.is_empty() {
        format!(
            "启动失败：企业微信启动后立即退出，日志里没有更多信息。日志：{}",
            log.display()
        )
    } else {
        format!("启动失败：{detail}。日志：{}", log.display())
    }
}
/// 启动一个 Wine 程序，但先观察一小段时间再宣布成功：旧实现只要进程创建成功就返回
/// 成功、并且丢弃标准错误，于是「升级后打不开」在界面上没有任何提示。输出写进
/// `launch.log`，用户在事后也能看到原因。
fn launch_checked(
    paths: &Paths,
    settings: &WineSettings,
    program: &Path,
    log: &Path,
) -> Result<(), String> {
    let mut command = wine_command(paths, settings);
    command.arg(program);
    let mut child = spawn_logged(command, log)?;
    let start = std::time::Instant::now();
    while start.elapsed() < LAUNCH_GRACE {
        if child.try_wait().map_err(error)?.is_some() {
            // 版本冲突不会自愈：直接给出「清理残留会话」的提示，不必再等。
            if !version_mismatch(&fs::read_to_string(log).unwrap_or_default()) {
                // 引导进程把工作交接给真正的进程后自己退出（企业微信会这样自我重启），
                // 这时 prefix 里很快能看到主程序，不算失败。
                for _ in 0..POST_EXIT_CONFIRM_POLLS {
                    if running(&paths.prefix) {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            return Err(launch_failure(&paths.prefix, log));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
/// 辅助进程（标题栏修复）的失败不应该挡住企业微信本身，所以只记录日志、不判失败。
fn launch_detached_logged(
    paths: &Paths,
    settings: &WineSettings,
    program: &Path,
    log: &Path,
) -> Result<(), String> {
    let mut command = wine_command(paths, settings);
    command.arg(program);
    let mut child = spawn_logged(command, log)?;
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
        require_chinese_locale()?;
        let log = prepare_log(&paths, "launch.log")?;
        if settings.titlebar_fix {
            let helper = write_helper(&paths)?;
            let helper_log = prepare_log(&paths, "titlebar.log")?;
            launch_detached_logged(&paths, &settings, &helper, &helper_log)?;
        }
        launch_checked(&paths, &settings, &paths.prefix.join(EXE), &log)
    })
    .await
    .map_err(error)?
}

/// 给这个 prefix 里的某个进程发信号。发之前用 /proc 重新确认它还属于本 prefix（同一个
/// Wine 进程、`WINEPREFIX` 指向同一个目录）：进程可能已经退出、PID 也可能被复用，
/// 不能凭旧快照开枪。固定 argv 调 `/bin/kill`，不经 shell。
fn signal_session_process(paths: &Paths, pid: i32, signal: &str, log: &Path) -> bool {
    let environ = Path::new("/proc").join(pid.to_string()).join("environ");
    let argv: Vec<String> = fs::read(Path::new("/proc").join(pid.to_string()).join("cmdline"))
        .unwrap_or_default()
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| {
            String::from_utf8_lossy(arg)
                .trim_matches('"')
                .replace('\\', "/")
                .to_ascii_lowercase()
        })
        .collect();
    if argv.is_empty()
        || !environ_matches_prefix(&environ, &paths.prefix)
        || !is_wine_process(pid, &argv)
    {
        return false;
    }
    let mut cmd = Command::new("/bin/kill");
    cmd.arg(signal).arg(pid.to_string());
    run(cmd, log, Duration::from_secs(15)).is_ok()
}
/// `wineserver -k` 连不上时（升级后残留的旧服务端协议不一致，`-k` 同样会被拒绝；
/// 或会话已经卡死）按 PID 兜底：先 TERM 再 KILL，每一步都复核进程确实消失。
fn kill_session_remainder(paths: &Paths, log: &Path) -> Result<(), String> {
    for (signal, polls) in [("-TERM", 20), ("-KILL", 20)] {
        for process in prefix_processes(&paths.prefix) {
            signal_session_process(paths, process.pid, signal, log);
        }
        for _ in 0..polls {
            if prefix_processes(&paths.prefix).is_empty() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    let left = prefix_processes(&paths.prefix);
    if left.is_empty() {
        return Ok(());
    }
    Err(format!(
        "该 Wine 环境仍有 {} 个进程未能结束（PID：{}），请在系统监视器中结束它们后重试",
        left.len(),
        left.iter()
            .map(|process| process.pid.to_string())
            .collect::<Vec<_>>()
            .join("、")
    ))
}
fn stopped_message(wecom: bool) -> String {
    if wecom {
        "企业微信已强制停止；未保存的输入可能丢失".into()
    } else {
        "已清理该 Wine 环境的残留后台进程，现在可以重新打开企业微信".into()
    }
}
/// 强制停止企业微信：按 prefix 终结整场 Wine 会话，而不是按进程名杀进程，因此不会
/// 影响用户在其它 Wine 环境里运行的程序。先 `wineserver -k`（SIGINT），仍在运行则
/// `-k9`；升级后残留的旧 wineserver 连 `-k` 都拒绝，所以最后按 PID 兜底。每一步都用
/// /proc 复核：只要这个 prefix 里还有进程（主程序、辅助进程或 wineserver）就继续 ——
/// 主程序退出后残留的后台正是「升级后打不开」的原因。
pub async fn stop() -> Result<String, String> {
    let busy = Busy::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _busy = busy;
        let paths = Paths::new()?;
        let processes = prefix_processes(&paths.prefix);
        if processes.is_empty() {
            return Ok("企业微信未在运行".into());
        }
        let wecom = processes.iter().any(is_wecom_process);
        let settings = paths.settings()?;
        let log = paths.root.join("operation.log");
        fs::create_dir_all(&paths.root).map_err(error)?;
        check_local_path(&paths.home, &log)?;
        let server = wineserver_path(&settings)?;
        for (signal, polls) in [("-k", 40), ("-k9", 20)] {
            let mut cmd = Command::new(&server);
            cmd.env("WINEPREFIX", &paths.prefix).arg(signal);
            // 服务端已经退出时 wineserver 会以非 0 状态返回，这不是失败，继续等进程消失。
            let _ = run(cmd, &log, Duration::from_secs(60));
            for _ in 0..polls {
                if prefix_processes(&paths.prefix).is_empty() {
                    return Ok(stopped_message(wecom));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        kill_session_remainder(&paths, &log)?;
        Ok(stopped_message(wecom))
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
pub async fn stop_windows_application() -> Result<String, String> {
    stop().await
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
            version_updated_at_unix_seconds: None,
            version_updated_at_source: None,
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
    fn release_version_time_must_be_paired_with_its_source() {
        let mut r = release_fixture();
        // 只有时间、没有来源：拒绝（不能凭空捏造一个来源）。
        r.version_updated_at_unix_seconds = Some(1_700_000_000);
        assert!(r.validate().is_err());
        // 成对出现：接受。
        r.version_updated_at_source = Some(crate::feed::VersionUpdatedAtSource::ServerModified);
        assert!(r.validate().is_ok());
        // 时间为 0 视为无效。
        r.version_updated_at_unix_seconds = Some(0);
        assert!(r.validate().is_err());
        // 只有来源、没有时间：同样拒绝。
        r.version_updated_at_unix_seconds = None;
        assert!(r.validate().is_err());
        // 都缺失是合法的（旧 feed 没有这个字段）。
        r.version_updated_at_source = None;
        assert!(r.validate().is_ok());
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
    /// 升级 Wine 后旧 wineserver 残留时，客户端打印的就是这一段
    /// （server.c 的 server_protocol_error("version mismatch …")）。启动失败提示必须
    /// 认得出它，否则「打不开」仍然只是一句没有信息量的报错。
    #[test]
    fn version_mismatch_launch_errors_are_recognised() {
        let sample = "wine client error:3c: version mismatch 1100/1102.\n\
                      Your wineserver binary was not upgraded correctly,\n";
        assert!(version_mismatch(sample));
        assert!(version_mismatch("VERSION MISMATCH"));
        assert!(!version_mismatch("wine: cannot find L\"C:\\\\missing.exe\""));
        assert_eq!(last_log_line("a\n\n  b  \n"), "b");
        assert_eq!(last_log_line("\n \n"), "");
    }
    /// Wine 11.20 的 GDI 只读 HKLM 的 SystemLink（dlls/win32u/font.c:105 的
    /// `system_link_keyW` 以 `\Registry\Machine` 开头）。写回 HKCU 等于彻底失效——
    /// 这正是中文方框的根因，所以给它一条断言，别让后人改回去。
    #[test]
    fn font_link_writes_the_machine_key_wine_reads() {
        assert_eq!(
            FONT_LINK_KEY,
            r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink"
        );
        assert_eq!(
            LEGACY_FONT_LINK_KEY,
            r"HKCU\Software\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink"
        );
    }
    /// 反效果回归：宿主上已经存在的字族（Tahoma / Arial / MS Sans Serif / MS Shell Dlg）
    /// 不能出现在要写 SystemLink 的名单里 —— 实测把它们写进 HKLM 会顶掉 Wine 自己的
    /// fontconfig 回退，把 GB2312 场景从「有中文字形」变成 0xFFFF。另外 wineboot 自带的
    /// HKLM `Tahoma` 默认条目不属于旧版本的死数据，只清 HKCU 那一份。
    #[test]
    fn font_link_never_targets_host_owned_families() {
        for face in FONT_LINK_HARMFUL_FACES
            .iter()
            .chain(LEGACY_FONT_LINK_FACES.iter())
        {
            assert!(
                !FONT_LINK_FACES.contains(face),
                "{face} 在宿主上存在，写 SystemLink 会让中文变差"
            );
        }
        assert!(!FONT_LINK_HARMFUL_FACES.contains(&"Tahoma"));
        assert!(LEGACY_FONT_LINK_FACES.contains(&"Tahoma"));
    }
    fn wait_for_prefix(prefix: &Path) -> Vec<SessionProcess> {
        for _ in 0..200 {
            let found = prefix_processes(prefix);
            if !found.is_empty() {
                return found;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Vec::new()
    }
    /// 轮询到「这些进程都被识别」为止：bash 启动有快慢，不能一看到有结果就断言。
    fn wait_for_prefix_pids(prefix: &Path, pids: &[i32]) -> Vec<SessionProcess> {
        let mut found = Vec::new();
        for _ in 0..200 {
            found = prefix_processes(prefix);
            if pids.iter().all(|pid| found.iter().any(|p| p.pid == *pid)) {
                return found;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        found
    }
    /// Wine 客户端把 Windows 程序路径放在 `wine` 之后的参数里
    /// （`wine /prefix/drive_c/.../WXWork.exe`），所以判定必须看整条命令行：
    /// 只看 argv[0] 会在这种启动形式下把正在运行的企业微信判成「没在运行」。
    #[test]
    fn wecom_predicate_matches_the_wine_loader_form() {
        let loader = SessionProcess {
            pid: 1,
            argv: vec![
                "/opt/wine-devel/bin/wine".into(),
                "/home/u/.local/share/wineprefixes/wecom/drive_c/program files (x86)/wxwork/wxwork.exe".into(),
            ],
        };
        assert!(is_wecom_process(&loader));
        let helper = SessionProcess {
            pid: 2,
            argv: vec!["c:/windows/system32/services.exe".into()],
        };
        assert!(!is_wecom_process(&helper));
    }
    /// 冒充 Wine 进程：会话识别的主判据是 `/proc/<pid>/exe` 的文件名含 wine，所以要造一个
    /// 「文件名带 wine、能长期运行」的可执行文件。coreutils 的 `sleep` 会校验调用名
    /// （复制改名后直接 `unknown program` 退出，测试会假绿），因此改为复制解释器：
    /// python3（或退而求其次 node）不校验自己的文件名，且系统上基本都有。
    fn fake_wine_process(prefix: impl AsRef<std::ffi::OsStr>) -> Command {
        static FAKE: OnceLock<(PathBuf, &'static str, &'static str)> = OnceLock::new();
        let (path, flag, script) = FAKE.get_or_init(|| {
            let path = std::env::temp_dir()
                .join(format!("umanager-wine-fake-{}", std::process::id()));
            for (name, flag, script) in [
                ("python3", "-c", "import time; time.sleep(120)"),
                ("node", "-e", "setTimeout(()=>{},120000)"),
            ] {
                if let Ok(source) = fs::canonicalize(Path::new("/usr/bin").join(name)) {
                    if fs::copy(&source, &path).is_ok() {
                        return (path, flag, script);
                    }
                }
            }
            panic!("需要 python3 或 node 才能冒充 Wine 进程");
        });
        let mut command = Command::new(path);
        command.arg(*flag).arg(*script).env("WINEPREFIX", prefix);
        command
    }
    /// /proc 会话识别：只认「Wine 进程 + `WINEPREFIX` 指向同一个目录」。`running()` 只认
    /// 企业微信主程序，残留的 wineserver / 辅助进程不算「企业微信在运行」，但会被 `stop()`
    /// 清掉。四条边界一起锁定：尾斜杠写法（Wine 不做规范化）、别的 prefix、**同名 prefix
    /// 但非 Wine 的进程**（用户 `export WINEPREFIX=…` 后启动的终端/UManager 自己），以及
    /// **命令行里恰好有 `*.exe` 的普通命令**（`tail -f x.exe` 这类，只有 exe 读不到时才该
    /// 退回 argv 判据）。
    #[test]
    fn prefix_scan_is_exact_and_only_wecom_counts_as_running() {
        let f = Fixture::new();
        let prefix = f.0.join("prefix");
        fs::create_dir_all(&prefix).unwrap();
        let decoy_file = f.0.join("decoy.exe");
        fs::write(&decoy_file, b"not a windows program").unwrap();
        let mut mine = fake_wine_process(&prefix).spawn().unwrap();
        let mut slash = fake_wine_process(format!("{}/", prefix.display()))
            .spawn()
            .unwrap();
        let mut other = fake_wine_process(f.0.join("other")).spawn().unwrap();
        let mut foreign = Command::new("sleep")
            .arg("120")
            .env("WINEPREFIX", &prefix)
            .spawn()
            .unwrap();
        let mut decoy = Command::new("tail")
            .arg("-f")
            .arg(&decoy_file)
            .env("WINEPREFIX", &prefix)
            .spawn()
            .unwrap();
        let found = wait_for_prefix_pids(&prefix, &[mine.id() as i32, slash.id() as i32]);
        assert!(
            found.iter().any(|p| p.pid == mine.id() as i32),
            "本 prefix 的进程应被识别：{found:?}"
        );
        assert!(
            found.iter().any(|p| p.pid == slash.id() as i32),
            "尾斜杠写法指的是同一个 prefix，也必须被识别：{found:?}"
        );
        assert!(
            !found.iter().any(|p| p.pid == other.id() as i32),
            "别的 prefix 的进程不能被算进来"
        );
        assert!(
            !found.iter().any(|p| p.pid == foreign.id() as i32),
            "只带 WINEPREFIX 环境变量、但不是 Wine 进程的（终端/UManager 自己）不能算会话"
        );
        assert!(
            !found.iter().any(|p| p.pid == decoy.id() as i32),
            "命令行里有 *.exe 的普通命令（tail -f x.exe）不能算会话"
        );
        assert!(!running(&prefix), "不是企业微信，不能算作正在运行");
        for child in [&mut mine, &mut slash, &mut other, &mut foreign, &mut decoy] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    /// 强制停止的兜底路径：`wineserver -k` 连不上（升级后残留的旧服务端）时按 PID 清理
    /// 本 prefix 的 Wine 进程，绝不碰别的 Wine 环境，也不碰只是继承了 `WINEPREFIX`
    /// 环境变量、或命令行里带 `*.exe` 的普通进程（那可能是 UManager 自己或用户的命令）。
    #[test]
    fn stop_remainder_kills_only_this_prefixes_wine_processes() {
        let f = Fixture::new();
        let paths = Paths::at(f.0.clone()).unwrap();
        fs::create_dir_all(&paths.prefix).unwrap();
        let log = paths.root.join("operation.log");
        let decoy_file = f.0.join("decoy.exe");
        fs::write(&decoy_file, b"not a windows program").unwrap();
        let mut mine = fake_wine_process(&paths.prefix).spawn().unwrap();
        let mut other = fake_wine_process("/tmp/umanager-other-prefix")
            .spawn()
            .unwrap();
        let mut foreign = Command::new("sleep")
            .arg("120")
            .env("WINEPREFIX", &paths.prefix)
            .spawn()
            .unwrap();
        let mut decoy = Command::new("tail")
            .arg("-f")
            .arg(&decoy_file)
            .env("WINEPREFIX", &paths.prefix)
            .spawn()
            .unwrap();
        assert!(!wait_for_prefix(&paths.prefix).is_empty());
        kill_session_remainder(&paths, &log).unwrap();
        assert!(
            mine.try_wait().unwrap().is_some(),
            "本 prefix 的残留 Wine 进程应被结束"
        );
        assert!(
            other.try_wait().unwrap().is_none(),
            "别的 prefix 的进程不能被杀掉"
        );
        assert!(
            foreign.try_wait().unwrap().is_none(),
            "只继承 WINEPREFIX 的非 Wine 进程不能被杀掉"
        );
        assert!(
            decoy.try_wait().unwrap().is_none(),
            "命令行里带 *.exe 的普通命令不能被杀掉"
        );
        for child in [&mut other, &mut foreign, &mut decoy] {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    /// `launch_checked` 只在进程撑过观察窗口后才算成功，因此「启动后立即退出」必须
    /// 返回带日志路径的错误，而不是静默成功。
    #[test]
    fn immediately_exiting_launch_is_reported_with_the_log() {
        let f = Fixture::new();
        let paths = Paths::at(f.0.clone()).unwrap();
        fs::create_dir_all(&paths.prefix).unwrap();
        let log = f.0.join("launch.log");
        fs::write(
            &log,
            "wine client error:3c: version mismatch 1100/1102.\nOr maybe the wrong wineserver is still running?\n",
        )
        .unwrap();
        let failure = launch_failure(&paths.prefix, &log);
        assert!(failure.contains("版本不一致"), "{failure}");
        assert!(failure.contains("强制停止"), "{failure}");
        assert!(failure.contains(&log.display().to_string()), "{failure}");
    }
    /// Minimal PE-shaped fixture whose `VS_FIXEDFILEINFO` carries
    /// `major.minor.patch.build`, as `pe_version` reads it.
    fn version_fixture(version: (u16, u16, u16, u16)) -> Vec<u8> {
        let (major, minor, patch, build) = version;
        let mut bytes = b"MZpadding".to_vec();
        bytes.extend([0xbd, 4, 0xef, 0xfe, 0, 0, 1, 0]);
        bytes.extend(((u32::from(major) << 16) | u32::from(minor)).to_le_bytes());
        bytes.extend(((u32::from(patch) << 16) | u32::from(build)).to_le_bytes());
        bytes
    }
    /// Regression for 「实际已经更新完成了，但会卡在这」: the WeCom installer
    /// auto-starts 企业微信, so `wineserver -w` can stay blocked for the whole
    /// 30-minute timeout *after* the installation has landed. The settle wait must
    /// end as soon as its completion criterion holds, without waiting for the Wine
    /// session to die.
    #[test]
    fn settle_wait_ends_when_the_operation_already_landed() {
        let mut waiter = Command::new("sleep").arg("60").spawn().unwrap();
        let start = std::time::Instant::now();
        wait_for_settle(&mut waiter, Duration::from_secs(30), || true).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the settle wait must not block on a surviving Wine session"
        );
        // The waiter is killed and reaped, not left as a zombie.
        assert!(waiter.try_wait().unwrap().is_some());
    }
    /// The bounded wait still reports a timeout when the operation never lands,
    /// instead of hanging forever on a Wine session that will not exit.
    #[test]
    fn settle_wait_still_times_out_when_nothing_landed() {
        let mut waiter = Command::new("sleep").arg("60").spawn().unwrap();
        let error = wait_for_settle(&mut waiter, Duration::from_millis(300), || false).unwrap_err();
        assert!(error.contains("超时"), "{error}");
        assert!(waiter.try_wait().unwrap().is_some());
    }
    /// The completion probe only reports success for the target version, and only
    /// re-parses the (large) executable when its size or mtime changed.
    #[test]
    fn version_probe_lands_only_on_the_target_version() {
        let f = Fixture::new();
        let paths = Paths::at(f.0.clone()).unwrap();
        let exe = paths.prefix.join(EXE);
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, version_fixture((5, 0, 12, 6013))).unwrap();
        let mut probe = VersionProbe::new(&paths.prefix, Some("5.0.12.6018"));
        // Old version installed: not landed, and an unchanged file is not re-read.
        assert!(!probe.landed());
        assert!(!probe.landed());
        // The installer replaced the executable with the target build.
        let mut updated = version_fixture((5, 0, 12, 6018));
        updated.push(0); // a different size, so the change is detected on any fs
        fs::write(&exe, updated).unwrap();
        assert!(probe.landed());
        // …and it stays landed on the following polls.
        assert!(probe.landed());
        let mut other = VersionProbe::new(&paths.prefix, Some("5.0.12.9999"));
        assert!(!other.landed());
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
    /// Windows 交叉编译器：与 build.rs 同一优先级，本地可直接用仓库里的 zig shim。
    fn windows_compiler() -> PathBuf {
        if let Ok(explicit) = std::env::var("UMANAGER_MINGW_CC") {
            if !explicit.trim().is_empty() {
                return PathBuf::from(explicit);
            }
        }
        let shim = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tools/mingw-cc");
        if shim.is_file() {
            return shim;
        }
        PathBuf::from("x86_64-w64-mingw32-gcc")
    }
    /// 跑 Windows 程序需要显示环境：没有 DISPLAY 时用 xvfb-run 起一个临时的。
    fn wine_runner(wine: &str) -> Command {
        let mut command = if std::env::var_os("DISPLAY").is_some() {
            Command::new(wine)
        } else {
            let mut command = Command::new("xvfb-run");
            command.args(["-a", wine]);
            command
        };
        // winemenubuilder 会在宿主上创建菜单项/图标，测试环境里必须关掉。
        command
            .env("WINEDEBUG", "-all")
            .env("WINEDLLOVERRIDES", "winemenubuilder.exe=d");
        command
    }
    /// 中文回归（端到端）：在一次性 prefix 上真的应用配置，再跑两次探针。
    ///
    /// - `link` 模式跑在 **C.UTF-8**（英文区域）下：此时宿主上不存在的字族只能靠 HKLM
    ///   的 FontLink\SystemLink 拿到中文字形，**这一条才是「FontLink 写对路径」的判据**
    ///   ——把配置写回 HKCU 或整个键不存在时它必然失败（实测整键删除后 Microsoft YaHei /
    ///   SimSun / NSimSun / YaHei UI 全部回退到 Arial，0xFFFF）。
    /// - `zh` 模式跑在中文区域下：GB2312 字符集下所有界面字体都必须有中文字形，同时回归
    ///   「给 Tahoma / Arial 这类宿主既有字族写 SystemLink 会让中文变差」的反效果。
    ///
    /// 需要 wine、fonts-noto-cjk、Windows 交叉编译器（UMANAGER_MINGW_CC 或
    /// x86_64-w64-mingw32-gcc）和显示环境。建议在 xvfb-run 下运行：
    /// `xvfb-run -a cargo test --manifest-path src-tauri/Cargo.toml -- --ignored
    ///  chinese_font_link_reaches_wine_gdi`。
    #[test]
    #[ignore = "requires Wine, fonts-noto-cjk, a Windows cross-compiler and an isolated display"]
    fn chinese_font_link_reaches_wine_gdi() {
        let f = Fixture::new();
        let paths = Paths::at(f.0.clone()).unwrap();
        let settings = WineSettings::default();
        let log = f.0.join("operation.log");
        apply_settings(&paths, &settings, &log).unwrap();

        // 配置确实写进了 Wine 读取的那条路径。
        let mut query = wine_command(&paths, &settings);
        query.args(["reg", "query", FONT_LINK_KEY]);
        let output = query.output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success() && text.contains("NotoSansCJK"),
            "SystemLink 未写入 HKLM：{text}"
        );

        // 编译探针：GetGlyphIndicesW("中文") 出现 0xffff 就是方框。
        let probe = f.0.join("font-probe.exe");
        let status = Command::new(windows_compiler())
            .args(["-Os", "-static"])
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../tests/fixtures/font-probe.c"
            ))
            .arg("-o")
            .arg(&probe)
            .arg("-lgdi32")
            .status()
            .unwrap();
        assert!(status.success(), "font-probe.c 编译失败");

        // link：英文区域，只有 SystemLink 能救回那几个字族。
        let mut link = wine_runner(&settings.wine_binary);
        link.arg(&probe)
            .arg("link")
            .env("WINEPREFIX", &paths.prefix)
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .env("LC_MESSAGES", "C.UTF-8")
            .env_remove("LANGUAGE");
        let out = link.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success(),
            "HKLM SystemLink 没有生效（英文区域下中文应为方框）：{text}"
        );

        // zh：真实配置下的中文区域，GB2312 下所有界面字体都不能是方框。
        let mut zh = wine_runner(&settings.wine_binary);
        zh.arg(&probe).arg("zh").env("WINEPREFIX", &paths.prefix);
        apply_chinese_locale(&mut zh);
        let out = zh.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success(),
            "中文字形缺失（方框）或字体链接反效果：{text}"
        );
        for face in ["Microsoft YaHei", "SimSun", "Tahoma", "cs=134", "cs=1"] {
            assert!(text.contains(face), "探针没有覆盖 {face}：{text}");
        }
    }
}
