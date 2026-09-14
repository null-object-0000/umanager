// Read-only detection of masked systemd **user** units.
//
// A Debian package that ships a user service is masked by its own `postrm` while
// it is being removed (`deb-systemd-helper --user mask`), and unmasked again by
// the matching `postinst`. That pairing leaks whenever the package never reaches
// a clean `configure`: the mask lands in `/etc/systemd/user` (or
// `~/.config/systemd/user`) as a symlink to `/dev/null`, and because a `/etc`
// unit outranks the package's own `/usr/lib/systemd/user` unit, every later
// start attempt fails silently.
//
// Concretely: docker-desktop failed to configure because it wanted
// `docker-ce-cli` from a repository that was not configured. Running
// `apt-get install -f` then *removed* it (masking the service), and once the
// missing repository was added and the package finally configured, the leftover
// mask meant clicking the icon did nothing at all.
//
// This module only reads the filesystem and never changes it: it reports the
// residue so the UI can tell the user which single command clears it.

use std::path::{Path, PathBuf};

/// What a masked unit symlink points at. `systemctl mask` links to `/dev/null`.
const MASK_TARGET: &str = "/dev/null";

/// systemd's `Type=user` unit search path, highest precedence first. A mask must
/// live here: masking never touches the package-owned `/usr/lib/systemd/user`.
const USER_UNIT_DIRS: [&str; 2] = ["/etc/systemd/user", ".config/systemd/user"];

/// The directory a package installs its own user units into. Its presence is the
/// evidence that the application — and not the user — is the author of the unit,
/// so a leftover mask on it is always package residue rather than a deliberate
/// choice, and we can derive the unit name from the package name.
pub(crate) const PACKAGE_USER_UNIT_DIR: &str = "/usr/lib/systemd/user";

/// Advisory for one application whose own user service is still masked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaskedUserUnit {
    pub(crate) unit_name: String,
    /// The stray symlink, so the user can see exactly what is wrong.
    pub(crate) mask_path: PathBuf,
}

impl MaskedUserUnit {
    /// User-facing explanation plus the one command that fixes it. Named rather
    /// than deleted so the user stays in control of their own system.
    pub(crate) fn warning(&self, display_name: &str) -> String {
        format!(
            "{display_name} 的服务被屏蔽（systemd user 单元 {} 被 mask），点图标不会有任何反应。\
这通常是该软件上一次卸载遗留下来的。请在终端执行 sudo rm -f {} 后重新登录，或执行 systemctl --user daemon-reload \
再重新登录。",
            self.unit_name,
            self.mask_path.display()
        )
    }
}

/// The package-shipped user unit of `package_name`, if it has one. Debian names
/// the unit after the package in the common case (`docker-desktop` →
/// `docker-desktop.service`); a package that does not follow that convention
/// simply yields `None` and is not reported.
pub(crate) fn unit_name_for_package(package_name: &str) -> Option<String> {
    let conventional = format!("{package_name}.service");
    is_plain_unit_name(&conventional).then_some(conventional)
}

/// Whether `unit_name` is a user unit shipped by an installed package.
pub(crate) fn package_ships_user_unit(unit_name: &str) -> bool {
    masked_unit_path(unit_name, Path::new(PACKAGE_USER_UNIT_DIR)).is_some()
}

/// Locates the mask that applies to `unit_name`, searching every user unit
/// directory in precedence order. Only a symlink to `/dev/null` counts — an
/// ordinary file, or a symlink to a real unit, is a user override and is left
/// alone.
pub(crate) fn masked_unit_path(unit_name: &str, user_unit_dir: &Path) -> Option<PathBuf> {
    let candidate = user_unit_dir.join(unit_name);
    let target = std::fs::read_link(&candidate).ok()?;
    (target == Path::new(MASK_TARGET)).then_some(candidate)
}

/// Every masked unit visible in the user unit directories, paired with the mask
/// symlink that applies to it. `None` means the user unit directories could not
/// be read at all: a failed probe must never be reported as a detected problem.
/// Precedence order decides the winner when the same unit is masked twice.
pub(crate) fn masked_unit_names(home: &Path) -> Option<Vec<(String, PathBuf)>> {
    collect_masked_units(&user_unit_dirs(home))
}

/// The scanning half of [`masked_unit_names`], separated so tests can point it at
/// a temporary directory instead of the machine's real systemd configuration.
fn collect_masked_units(directories: &[PathBuf]) -> Option<Vec<(String, PathBuf)>> {
    let mut masked: Vec<(String, PathBuf)> = Vec::new();
    let mut readable = false;
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        readable = true;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_plain_unit_name(&name) || masked.iter().any(|(seen, _)| *seen == name) {
                continue;
            }
            if let Some(path) = masked_unit_path(&name, directory) {
                masked.push((name, path));
            }
        }
    }
    readable.then_some(masked)
}

fn user_unit_dirs(home: &Path) -> Vec<PathBuf> {
    USER_UNIT_DIRS
        .iter()
        .map(|directory| {
            if directory.starts_with('/') {
                PathBuf::from(directory)
            } else {
                home.join(directory)
            }
        })
        .collect()
}

/// Defensive filter, mirroring the helper's `valid_package_name` discipline: we
/// only ever join a name we generated or one that looks like a plain unit file,
/// never an arbitrary path component from the filesystem. A stem is required, so
/// a bare `.service` (what an empty package name would produce) is rejected.
fn is_plain_unit_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .strip_suffix(".service")
            .is_some_and(|stem| !stem.is_empty() && stem.len() <= 120)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "umanager-systemd-units-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn detects_a_mask_symlink_pointing_at_dev_null() {
        let directory = temp_dir("masked");
        std::os::unix::fs::symlink(MASK_TARGET, directory.join("docker-desktop.service")).unwrap();
        assert_eq!(
            masked_unit_path("docker-desktop.service", &directory),
            Some(directory.join("docker-desktop.service"))
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn ignores_real_units_and_unrelated_symlinks() {
        let directory = temp_dir("real");
        std::fs::write(directory.join("real.service"), "[Unit]\n").unwrap();
        std::os::unix::fs::symlink(
            "/usr/lib/systemd/user/override.service",
            directory.join("override.service"),
        )
        .unwrap();
        assert_eq!(masked_unit_path("real.service", &directory), None);
        assert_eq!(masked_unit_path("override.service", &directory), None);
        assert_eq!(masked_unit_path("absent.service", &directory), None);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn collects_every_masked_unit_with_its_mask_path() {
        let directory = temp_dir("collect");
        std::os::unix::fs::symlink(MASK_TARGET, directory.join("docker-desktop.service")).unwrap();
        std::os::unix::fs::symlink(MASK_TARGET, directory.join("other.service")).unwrap();
        std::fs::write(directory.join("active.service"), "[Unit]\n").unwrap();
        // Not a unit: must not be reported.
        std::os::unix::fs::symlink(MASK_TARGET, directory.join("notes.txt")).unwrap();

        let mut masked = collect_masked_units(&[directory.clone()]).unwrap();
        masked.sort();
        assert_eq!(
            masked,
            vec![
                (
                    "docker-desktop.service".to_owned(),
                    directory.join("docker-desktop.service")
                ),
                ("other.service".to_owned(), directory.join("other.service")),
            ]
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_same_unit_masked_twice_is_reported_once() {
        let high = temp_dir("precedence-high");
        let low = temp_dir("precedence-low");
        for directory in [&high, &low] {
            std::os::unix::fs::symlink(MASK_TARGET, directory.join("dup.service")).unwrap();
        }
        assert_eq!(
            collect_masked_units(&[high.clone(), low.clone()]),
            Some(vec![("dup.service".to_owned(), high.join("dup.service"))])
        );
        let _ = std::fs::remove_dir_all(&high);
        let _ = std::fs::remove_dir_all(&low);
    }

    #[test]
    fn an_unreadable_directory_is_not_reported_as_detected() {
        let missing = std::env::temp_dir().join("umanager-systemd-units-does-not-exist");
        assert_eq!(collect_masked_units(&[missing]), None);
        // A directory that exists but holds no mask is an empty result, not `None`.
        let empty = temp_dir("empty");
        assert_eq!(collect_masked_units(&[empty.clone()]), Some(Vec::new()));
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn only_conventional_unit_names_are_derived_from_a_package() {
        assert_eq!(
            unit_name_for_package("docker-desktop"),
            Some("docker-desktop.service".to_owned())
        );
        // Anything that is not a plain name never reaches a filesystem join.
        assert_eq!(unit_name_for_package("../../etc/passwd"), None);
        assert_eq!(unit_name_for_package("has space"), None);
        assert_eq!(unit_name_for_package(""), None);
        // A bare extension is not a unit either.
        assert!(!is_plain_unit_name(".service"));
        assert!(!is_plain_unit_name("notes.txt"));
        assert!(is_plain_unit_name("docker-desktop.service"));
    }

    #[test]
    fn warning_names_the_unit_and_the_exact_command() {
        let unit = MaskedUserUnit {
            unit_name: "docker-desktop.service".to_owned(),
            mask_path: PathBuf::from("/etc/systemd/user/docker-desktop.service"),
        };
        let warning = unit.warning("Docker Desktop");
        assert!(warning.contains("Docker Desktop"));
        assert!(warning.contains("docker-desktop.service"));
        assert!(warning.contains("sudo rm -f /etc/systemd/user/docker-desktop.service"));
    }
}
