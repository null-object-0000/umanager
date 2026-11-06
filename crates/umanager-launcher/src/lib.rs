//! Ordinary-user launch adapter for the two managed Antigravity archives.
//! No shell, network requests, privilege elevation or changes to vendor files.
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
mod paths;

fn executable(id: &str) -> Option<&'static str> {
    match id {
        "antigravity" => Some("Antigravity-x64/antigravity"),
        "antigravity-ide" => Some("Antigravity IDE/antigravity-ide"),
        _ => None,
    }
}

pub fn desktop_exec(home: &Path, id: &str) -> Result<Option<String>, String> {
    if executable(id).is_none() {
        return Ok(None);
    }
    let launcher = home.join(".local/share/umanager/bin/umanager-antigravity-launcher");
    Ok(Some(format!(
        "{} --launch-antigravity {id}",
        paths::desktop_quote(&launcher)?
    )))
}

fn target(home: &Path, id: &str) -> Result<PathBuf, String> {
    let relative = executable(id).ok_or("不支持的归档启动目标")?;
    let root = home.join(".local/share/umanager/apps").join(id);
    let state = root.join("installed.json");
    paths::check_path(home, &state)?;
    if fs::metadata(&state).map_err(|e| e.to_string())?.len() > 16384 {
        return Err("安装记录过大".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(state).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let hash = value["sha256"]
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or("安装摘要无效")?;
    let path = root.join(hash).join(relative);
    paths::check_path(home, &path)?;
    if !path.is_file() {
        return Err("归档程序缺失，请重新安装".into());
    }
    Ok(path)
}

fn setting(schema: &str, key: &str) -> Option<String> {
    let output = Command::new("/usr/bin/gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 4096 {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}
fn variant_string(value: &str) -> Option<&str> {
    value.strip_prefix('\'')?.strip_suffix('\'')
}
fn endpoint(host: &str, port: &str) -> Option<String> {
    let host = variant_string(host)?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]_".contains(&b))
    {
        return None;
    }
    let port: u16 = port.parse().ok()?;
    if port == 0 {
        return None;
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Some(format!("http://{host}:{port}"))
}
fn system_proxy() -> Option<String> {
    if setting("org.gnome.system.proxy", "mode")?.as_str() != "'manual'" {
        return None;
    }
    endpoint(
        &setting("org.gnome.system.proxy.https", "host")?,
        &setting("org.gnome.system.proxy.https", "port")?,
    )
    .or_else(|| {
        endpoint(
            &setting("org.gnome.system.proxy.http", "host")?,
            &setting("org.gnome.system.proxy.http", "port")?,
        )
    })
}
fn configured_proxy(home: &Path) -> Option<String> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let path = dir.join("io.github.umanager.app/network.json");
    if fs::metadata(&path).ok()?.len() > 16384 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if value["proxyEnabled"] != true {
        return None;
    }
    let proxy = value["proxyUrl"].as_str()?.trim();
    let url = url::Url::parse(proxy).ok()?;
    // Go's HTTP transport supports HTTP(S) CONNECT and SOCKS5 proxies.
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") || url.host_str().is_none()
    {
        return None;
    }
    Some(proxy.to_owned())
}
fn proxy_environment(
    inherited: &BTreeMap<String, String>,
    fallback: Option<&str>,
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for (upper, lower) in [("HTTP_PROXY", "http_proxy"), ("HTTPS_PROXY", "https_proxy")] {
        if let Some(value) = inherited
            .get(upper)
            .filter(|s| !s.is_empty())
            .or_else(|| inherited.get(lower).filter(|s| !s.is_empty()))
            .map(String::as_str)
            .or(fallback)
        {
            result.insert(upper.into(), value.into());
            result.insert(lower.into(), value.into());
        }
    }
    let mut bypass = inherited
        .get("NO_PROXY")
        .or_else(|| inherited.get("no_proxy"))
        .cloned()
        .unwrap_or_default();
    for host in ["localhost", "127.0.0.1", "::1"] {
        if !bypass.split(',').any(|v| v.trim() == host) {
            if !bypass.is_empty() {
                bypass.push(',');
            }
            bypass.push_str(host);
        }
    }
    result.insert("NO_PROXY".into(), bypass.clone());
    result.insert("no_proxy".into(), bypass);
    result
}

/// Standalone entry point: no Tauri, single-instance plugin or GUI initialization.
pub fn dispatch() -> Option<Result<(), String>> {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next()?;
    if mode == "--refresh-antigravity-launchers" {
        return Some(if args.next().is_none() {
            refresh_installed()
        } else {
            Err("刷新入口不接受额外参数".into())
        });
    }
    if mode != "--launch-antigravity" {
        return None;
    }
    Some((|| {
        let id = args
            .next()
            .and_then(|s| s.into_string().ok())
            .ok_or("缺少归档应用标识")?;
        if args.next().is_some() {
            return Err("归档启动入口不接受程序参数".into());
        }
        let home = paths::home()?;
        let path = target(&home, &id)?;
        let inherited = std::env::vars().collect();
        let fallback = configured_proxy(&home).or_else(system_proxy);
        let error = Command::new(path)
            .envs(proxy_environment(&inherited, fallback.as_deref()))
            .exec();
        Err(format!("无法启动 Antigravity：{error}"))
    })())
}

pub fn prepare_desktop(home: &Path, text: &str, id: &str) -> Result<String, String> {
    if executable(id).is_none() {
        return Ok(text.to_owned());
    }
    updated_desktop(home, text, id)
}

fn updated_desktop(home: &Path, text: &str, id: &str) -> Result<String, String> {
    if !text
        .lines()
        .any(|s| s == format!("X-UManager-Application={id}"))
    {
        return Err("保留非 UManager 的桌面入口".into());
    }
    let exec = desktop_exec(home, id)?.ok_or("不支持的归档应用")?;
    let class = if id == "antigravity" {
        "Antigravity"
    } else {
        "Antigravity IDE"
    };
    if !text.lines().any(|line| line == "[Desktop Entry]") {
        return Err("桌面入口缺少 Desktop Entry 分组".into());
    }
    let mut lines = Vec::new();
    let mut main = false;
    let mut inserted = false;
    for line in text.lines().chain(std::iter::once("[__end__]")) {
        if line.starts_with('[') {
            if main && !inserted {
                lines.push(format!("Exec={exec}"));
                lines.push(format!("StartupWMClass={class}"));
                inserted = true;
            }
            main = line == "[Desktop Entry]";
        }
        if line == "[__end__]" {
            break;
        }
        if main
            && ["Exec=", "StartupWMClass=", "NoDisplay="]
                .iter()
                .any(|prefix| line.starts_with(prefix))
        {
            continue;
        }
        lines.push(line.to_owned());
    }
    Ok(lines.join("\n") + "\n")
}

// Wayland app_id matches the native product name, rather than the prefixed
// UManager launcher name. A hidden alias supplies its icon without duplicate
// app-menu entries, or any mutation of the vendor's signed archive contents.
pub fn sync_icon_alias(home: &Path, id: &str, desktop: &str) -> Result<(), String> {
    if executable(id).is_none() {
        return Ok(());
    }
    let alias = home
        .join(".local/share/applications")
        .join(format!("{id}.desktop"));
    paths::check_path(home, &alias)?;
    match fs::read_to_string(&alias) {
        Ok(old)
            if !old
                .lines()
                .any(|s| s == format!("X-UManager-Application={id}")) =>
        {
            return Err("保留同名第三方桌面入口，未覆盖图标关联".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let contents = updated_desktop(home, desktop, id)?.replacen(
        "[Desktop Entry]\n",
        "[Desktop Entry]\nNoDisplay=true\n",
        1,
    );
    paths::atomic_write(home, &alias, contents.as_bytes())
}
pub fn remove_icon_alias(home: &Path, id: &str) -> Result<(), String> {
    if executable(id).is_none() {
        return Ok(());
    }
    let alias = home
        .join(".local/share/applications")
        .join(format!("{id}.desktop"));
    paths::check_path(home, &alias)?;
    if fs::read_to_string(&alias).is_ok_and(|s| {
        s.lines()
            .any(|s| s == format!("X-UManager-Application={id}"))
    }) {
        fs::remove_file(alias).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn refresh_installed() -> Result<(), String> {
    let home = paths::home()?;
    let mut errors = Vec::new();
    for id in ["antigravity", "antigravity-ide"] {
        let path = home
            .join(".local/share/applications")
            .join(format!("umanager-{id}.desktop"));
        if !path.exists() {
            continue;
        }
        let result = (|| {
            target(&home, id)?;
            paths::check_path(&home, &path)?;
            if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 16384 {
                return Err("桌面入口过大".into());
            }
            let text = updated_desktop(
                &home,
                &fs::read_to_string(&path).map_err(|e| e.to_string())?,
                id,
            )?;
            paths::atomic_write(&home, &path, text.as_bytes())?;
            sync_icon_alias(&home, id, &text)
        })();
        if let Err(e) = result {
            errors.push(format!("{id}: {e}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_precedence_and_loopback_bypass() {
        let inherited = BTreeMap::from([
            ("https_proxy".into(), "http://explicit:8080".into()),
            ("NO_PROXY".into(), "company.test,10.0.0.0/8".into()),
        ]);
        let result = proxy_environment(&inherited, Some("http://system:7899"));
        assert_eq!(result["HTTPS_PROXY"], "http://explicit:8080");
        assert_eq!(result["HTTP_PROXY"], "http://system:7899");
        assert_eq!(
            result["no_proxy"],
            "company.test,10.0.0.0/8,localhost,127.0.0.1,::1"
        );
        assert_eq!(
            proxy_environment(&result, None)["NO_PROXY"],
            result["NO_PROXY"]
        );
        assert!(!proxy_environment(&BTreeMap::new(), None).contains_key("HTTPS_PROXY"));
    }
    #[test]
    fn system_endpoints_never_interpret_shell_or_invalid_ports() {
        assert_eq!(
            endpoint("'::1'", "7890").as_deref(),
            Some("http://[::1]:7890")
        );
        for host in [
            "''",
            "'localhost; touch x'",
            "'$(id)'",
            "'host/path'",
            "'user@host'",
        ] {
            assert!(endpoint(host, "7890").is_none());
        }
        for port in ["0", "-1", "65536", "80;id"] {
            assert!(endpoint("'localhost'", port).is_none());
        }
    }
    #[test]
    fn migration_preserves_icon_and_uses_shared_launcher() {
        let old = "[Desktop Entry]\nType=Application\nName=Google Antigravity\nExec=/old/binary\nIcon=/old/icon.png\nX-UManager-Application=antigravity\n";
        let new = updated_desktop(Path::new("/home/example"), old, "antigravity").unwrap();
        assert!(new.contains("--launch-antigravity antigravity\n"));
        assert!(new.contains("Icon=/old/icon.png\n"));
        assert!(new.contains("StartupWMClass=Antigravity\n"));
        assert!(!new.contains("HTTP_PROXY="));
        assert_eq!(
            new,
            updated_desktop(Path::new("/home/example"), &new, "antigravity").unwrap()
        );
        assert!(
            updated_desktop(
                Path::new("/home/example"),
                "[Desktop Entry]\nExec=custom\n",
                "antigravity"
            )
            .is_err()
        );
        assert!(
            desktop_exec(Path::new("/home/example"), "other-app")
                .unwrap()
                .is_none()
        );
        let action = old.to_owned() + "[Desktop Action Settings]\nExec=/custom/settings\n";
        let new_action =
            updated_desktop(Path::new("/home/example"), &action, "antigravity").unwrap();
        assert!(new_action.ends_with("[Desktop Action Settings]\nExec=/custom/settings\n"));
        assert_eq!(
            new_action,
            updated_desktop(Path::new("/home/example"), &new_action, "antigravity").unwrap()
        );
    }
    #[test]
    fn launch_state_and_icon_aliases_reject_unsafe_paths_and_preserve_custom_files() {
        let home =
            std::env::temp_dir().join(format!("umanager-launch-test-{}", std::process::id()));
        fs::create_dir_all(home.join(".local/share/umanager/apps/antigravity")).unwrap();
        fs::create_dir_all(home.join(".local/share/applications")).unwrap();
        let state = home.join(".local/share/umanager/apps/antigravity/installed.json");
        fs::write(&state, br#"{"sha256":"../../bad"}"#).unwrap();
        assert!(target(&home, "antigravity").is_err());
        assert!(target(&home, "other-app").is_err());
        for digit in ["a", "b"] {
            let hash = digit.repeat(64);
            let exe = state
                .parent()
                .unwrap()
                .join(&hash)
                .join("Antigravity-x64/antigravity");
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, b"installed executable").unwrap();
            fs::write(
                &state,
                serde_json::to_vec(&serde_json::json!({"sha256":hash})).unwrap(),
            )
            .unwrap();
            assert_eq!(target(&home, "antigravity").unwrap(), exe);
        }
        let exe = target(&home, "antigravity").unwrap();
        fs::remove_file(&exe).unwrap();
        std::os::unix::fs::symlink("/bin/true", &exe).unwrap();
        assert!(target(&home, "antigravity").is_err());
        let alias = home.join(".local/share/applications/antigravity.desktop");
        fs::write(&alias, "custom").unwrap();
        assert!(
            sync_icon_alias(&home, "antigravity", "X-UManager-Application=antigravity\n").is_err()
        );
        remove_icon_alias(&home, "antigravity").unwrap();
        assert_eq!(fs::read_to_string(&alias).unwrap(), "custom");
        fs::remove_file(&alias).unwrap();
        let original = "[Desktop Entry]\nIcon=/icon.png\nX-UManager-Application=antigravity\n";
        sync_icon_alias(&home, "antigravity", original).unwrap();
        assert!(
            fs::read_to_string(&alias)
                .unwrap()
                .contains("NoDisplay=true")
        );
        remove_icon_alias(&home, "antigravity").unwrap();
        assert!(!alias.exists());
        std::os::unix::fs::symlink(&state, &alias).unwrap();
        assert!(sync_icon_alias(&home, "antigravity", original).is_err());
        fs::remove_dir_all(home).unwrap();
    }
}
