//! Package-configuration helper. Root-only, no caller-controlled paths, profiles,
//! catalog fields, environment overrides, or shell commands. The ordinary
//! plan-based umanager-helper entry point is completely unchanged.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER: &str = "# UManager managed AppArmor profile v1.";
const PROFILES: [(&str, &str); 2] = [
    (
        "umanager-antigravity-app",
        include_str!("../../../packaging/apparmor/umanager-antigravity-app"),
    ),
    (
        "umanager-antigravity-ide",
        include_str!("../../../packaging/apparmor/umanager-antigravity-ide"),
    ),
];
const PARSER: &str = "/usr/sbin/apparmor_parser";

fn main() {
    let result = (|| {
        if std::env::args_os().skip(1).collect::<Vec<_>>() != ["configure"] {
            return Err("usage: umanager-apparmor configure".to_owned());
        }
        // SAFETY: geteuid takes no arguments and has no side effects.
        if unsafe { libc::geteuid() } != 0 {
            return Err("AppArmor package configuration requires root".to_owned());
        }
        configure()
    })();
    match result {
        Ok(message) => println!("umanager-apparmor: {message}"),
        Err(error) => {
            eprintln!("umanager-apparmor: {error}");
            std::process::exit(1);
        }
    }
}

fn configure() -> Result<&'static str, String> {
    // Older Ubuntu releases and systems without active AppArmor need no
    // userns exception. Do not leave syntax their older parsers cannot read.
    let restricted = fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
        .is_ok_and(|value| value.trim() == "1");
    let enabled = fs::read_to_string("/sys/module/apparmor/parameters/enabled")
        .is_ok_and(|value| value.trim() == "Y");
    if !restricted || !enabled || !Path::new("/sys/kernel/security/apparmor/profiles").exists() {
        return Ok("namespace restrictions inactive; no policy changes needed");
    }
    if !Path::new("/etc/apparmor.d/abi/4.0").is_file() || !Path::new(PARSER).is_file() {
        return Err("namespace restrictions are active but AppArmor 4+ tools are unavailable; install/repair apparmor and run dpkg --configure u-manager".into());
    }
    provision(Path::new("/etc/apparmor.d"), 0, &|args| {
        let output = Command::new(PARSER)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LC_ALL", "C")
            .output()
            .map_err(|e| format!("cannot run AppArmor parser: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "AppArmor parser failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    })?;
    Ok("Antigravity App and IDE sandbox policies installed and loaded")
}

fn secure_directory(path: &Path, owner: u32) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o022 != 0
    {
        return Err(format!(
            "policy directory must be owned by root and not writable by others: {}",
            path.display()
        ));
    }
    Ok(())
}

fn existing_policy(path: &Path, owner: u32) -> Result<Option<String>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o022 != 0
        || metadata.nlink() != 1
        || metadata.len() > 16 * 1024
    {
        return Err(format!("unsafe existing policy: {}", path.display()));
    }
    let contents = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if !contents.starts_with(MARKER) {
        return Err(format!(
            "refusing to replace administrator-owned custom policy: {}",
            path.display()
        ));
    }
    Ok(Some(contents))
}

fn write_stage(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = directory.join(format!(".{name}-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("cannot stage policy: {e}"))?;
    let result = (|| {
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.set_permissions(fs::Permissions::from_mode(0o644))
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

fn provision(
    directory: &Path,
    owner: u32,
    parser: &dyn Fn(&[String]) -> Result<(), String>,
) -> Result<(), String> {
    secure_directory(directory, owner)?;
    // Validate both destinations before changing either, preserving any custom
    // profiles (including the earlier per-user umanager-antigravity rule).
    let mut previous = Vec::new();
    for (name, _) in PROFILES {
        previous.push(existing_policy(&directory.join(name), owner)?);
    }
    let mut staged = Vec::new();
    let prepared = (|| {
        for (name, contents) in PROFILES {
            let path = write_stage(directory, name, contents.as_bytes())?;
            staged.push(path.clone());
            // No kernel changes until both embedded policies pass validation.
            parser(&[
                "-Q".into(),
                "-K".into(),
                path.to_string_lossy().into_owned(),
            ])?;
        }
        Ok::<_, String>(())
    })();
    if let Err(e) = prepared {
        for path in staged {
            let _ = fs::remove_file(path);
        }
        return Err(e);
    }
    for (index, (name, contents)) in PROFILES.iter().enumerate() {
        let target = directory.join(name);
        if previous[index].as_deref() == Some(contents) {
            fs::remove_file(&staged[index]).map_err(|e| e.to_string())?;
        } else {
            fs::rename(&staged[index], target).map_err(|e| e.to_string())?;
        }
    }
    // A kernel-load failure is a failed dpkg configure, never silent success.
    // The valid files remain for reboot/retry; the next configure is idempotent.
    let mut args = vec!["-r".into(), "-K".into()];
    args.extend(
        PROFILES
            .iter()
            .map(|(name, _)| directory.join(name).to_string_lossy().into_owned()),
    );
    parser(&args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    fn directory() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "umanager-apparmor-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    fn owner(path: &Path) -> u32 {
        fs::metadata(path).unwrap().uid()
    }
    #[test]
    fn configure_and_upgrade_load_only_bounded_embedded_policies() {
        let dir = directory();
        let calls = std::cell::RefCell::new(Vec::new());
        let parser = |args: &[String]| {
            calls.borrow_mut().push(args.to_vec());
            Ok(())
        };
        provision(&dir, owner(&dir), &parser).unwrap();
        provision(&dir, owner(&dir), &parser).unwrap();
        assert_eq!(calls.borrow().len(), 6);
        for (name, contents) in PROFILES {
            let path = dir.join(name);
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
            assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o644);
            assert_eq!(contents.matches("[0-9a-f]").count(), 64);
            assert!(contents.contains("flags=(unconfined)") && contents.contains("  userns,"));
            for forbidden in ["/**", "--no-sandbox", "sysctl", "capability sys_admin"] {
                assert!(!contents.contains(forbidden));
            }
        }
        assert_eq!(&calls.borrow()[2][..2], ["-r", "-K"]);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn custom_profiles_and_symlinks_are_preserved_before_any_write() {
        for symlink in [false, true] {
            let dir = directory();
            let target = dir.join(PROFILES[1].0);
            let original = dir.join("administrator-policy");
            fs::write(&original, b"custom profile").unwrap();
            if symlink {
                std::os::unix::fs::symlink(&original, &target).unwrap();
            } else {
                fs::write(&target, b"custom profile").unwrap();
            }
            assert!(
                provision(&dir, owner(&dir), &|_| panic!("parser must not be called")).is_err()
            );
            assert!(!dir.join(PROFILES[0].0).exists());
            assert_eq!(fs::read(original).unwrap(), b"custom profile");
            fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    fn validation_failure_keeps_old_files_and_load_failure_is_retryable() {
        let dir = directory();
        let calls = std::cell::Cell::new(0);
        let result = provision(&dir, owner(&dir), &|_| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err("syntax failed".into())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        assert!(
            provision(&dir, owner(&dir), &|args| if args[0] == "-r" {
                Err("kernel load failed".into())
            } else {
                Ok(())
            })
            .is_err()
        );
        provision(&dir, owner(&dir), &|_| Ok(())).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn refuses_untrusted_policy_directory() {
        let dir = directory();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(provision(&dir, owner(&dir), &|_| panic!("must not run")).is_err());
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
