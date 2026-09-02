use std::path::{Path, PathBuf};
use std::process::Command;
use umanager_catalog::Application;

const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";

/// Launch an installed managed desktop application via its `.desktop` entry.
///
/// This runs as the *current user* (no root, no Polkit) — launching an app is a
/// non-privileged action, unlike install/uninstall. Both system commands use a
/// fixed argv and never go through a shell:
///
///   * `/usr/bin/dpkg-query -L <package>` lists files owned by the package.
///   * `gio launch <desktop-file>` starts the picked launcher.
///
/// The package name originates from the signed catalog (resolved by
/// `application_id` in the command layer), not from free-form user input.
pub fn launch(app: &Application) -> Result<(), String> {
    let listing = list_package_files(&app.package_name)?;
    let desktop = select_desktop_file(&listing)
        .ok_or_else(|| format!("{} 未提供可启动的 .desktop 启动项", app.display_name))?;
    launch_desktop_file(&desktop, &app.display_name)
}

fn list_package_files(package_name: &str) -> Result<String, String> {
    let output = Command::new(DPKG_QUERY_BIN)
        .args(["-L", package_name])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C")
        .output()
        .map_err(|error| format!("无法执行 dpkg-query：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "dpkg-query 执行失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Pick the best launcher from a `dpkg-query -L` listing.
///
/// Prefers a plain GUI launcher over URL / protocol handlers, and system
/// (`/usr/share/applications`) entries over user-local ones. The returned path
/// is guaranteed to exist on disk.
pub(crate) fn select_desktop_file(listing: &str) -> Option<String> {
    let mut candidates: Vec<String> = listing
        .lines()
        .map(str::trim)
        .filter(|line| line.ends_with(".desktop") && line.contains("/applications/"))
        .map(str::to_owned)
        .collect();
    candidates.sort_by(|left, right| desktop_rank(left).cmp(&desktop_rank(right)));
    candidates
        .into_iter()
        .find(|path| Path::new(path).is_file())
}

/// Lower values sort first: `(is_handler, is_user_local, file_name)`.
fn desktop_rank(path: &str) -> (bool, bool, String) {
    let name = Path::new(path)
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_handler = name.contains("handler")
        || name.contains("protocol")
        || name.contains("url")
        || name.contains("uri");
    let is_user_local = path.to_lowercase().contains(".local/share/applications");
    (is_handler, is_user_local, name)
}

fn launch_desktop_file(desktop: &str, display_name: &str) -> Result<(), String> {
    let gio = find_gio()?;
    let status = Command::new(&gio)
        .arg("launch")
        .arg(desktop)
        .status()
        .map_err(|error| format!("无法启动 {display_name}：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        let code = status
            .code()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "未知".to_owned());
        Err(format!("gio launch 退出码 {code}"))
    }
}

fn find_gio() -> Result<PathBuf, String> {
    for candidate in ["/usr/bin/gio", "/usr/local/bin/gio", "/bin/gio"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err("未找到 gio（libglib2.0-bin），无法启动应用。请先安装 gio 后重试。".to_owned())
}

/// Only `https://` / `http://` URLs may be opened externally. Everything else
/// (file://, javascript:, gio://, …) is rejected so a crafted changelog link
/// can never ask `gio` to open a local path or a privileged scheme.
fn external_url_scheme_ok(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// Open an external URL in the user's default browser via `gio open`.
///
/// Runs as the *current user* (no Polkit). Only `https://` / `http://` URLs are
/// accepted, and the URL is passed as a single fixed argv — never through a
/// shell — so `gio` treats it as a literal argument rather than a command line.
pub fn open_url(url: &str) -> Result<(), String> {
    if !external_url_scheme_ok(url) {
        return Err("仅支持打开 http/https 链接".to_owned());
    }
    let gio = find_gio().map_err(|_| {
        "未找到 gio（libglib2.0-bin），无法打开外部链接。请先安装 gio 后重试。".to_owned()
    })?;
    let status = Command::new(&gio)
        .arg("open")
        .arg(url)
        .status()
        .map_err(|error| format!("无法打开链接：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        let code = status
            .code()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "未知".to_owned());
        Err(format!("gio open 退出码 {code}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_plain_system_launcher_over_url_handler() {
        let listing = concat!(
            "/usr/share/applications/code-url-handler.desktop\n",
            "/usr/share/applications/code.desktop\n",
            "/usr/share/doc/code/changelog.gz\n",
        );
        assert_eq!(
            select_desktop_file(listing),
            Some("/usr/share/applications/code.desktop".to_owned())
        );
    }

    #[test]
    fn prefers_system_entry_over_user_local() {
        let listing = concat!(
            "/home/user/.local/share/applications/wechat.desktop\n",
            "/usr/share/applications/wechat.desktop\n",
        );
        assert_eq!(
            select_desktop_file(listing),
            Some("/usr/share/applications/wechat.desktop".to_owned())
        );
    }

    #[test]
    fn ignores_non_desktop_files() {
        let listing = "/usr/share/applications/foo.png\n/usr/share/doc/foo/readme\n";
        assert_eq!(select_desktop_file(listing), None);
    }

    #[test]
    fn external_url_scheme_accepts_only_http_and_https() {
        assert!(external_url_scheme_ok("https://github.com/x/releases"));
        assert!(external_url_scheme_ok("http://example.com"));
        assert!(!external_url_scheme_ok("file:///etc/passwd"));
        assert!(!external_url_scheme_ok("javascript:alert(1)"));
        assert!(!external_url_scheme_ok("gio://foo"));
        assert!(!external_url_scheme_ok(""));
        assert!(!external_url_scheme_ok("HTTPS://example.com"));
    }

    #[test]
    fn open_url_rejects_non_http_schemes_before_touching_gio() {
        // These must fail on the scheme check alone, so the assertion is
        // independent of whether `gio` is installed on the test machine.
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("").is_err());
    }
}
