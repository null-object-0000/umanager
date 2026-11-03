// Read-only dependency pre-check for a downloaded `.deb`.
//
// UManager installs with a fixed `/usr/bin/dpkg --install` and intentionally
// does not resolve dependencies. To save the user from an install that fails
// half-way, this module reads the package `Pre-Depends` / `Depends` control
// fields and checks whether they are already satisfied by the locally installed
// set. It is advisory only: it never changes the immutable plan, never escalates
// privileges and never blocks an install itself — the UI surfaces the gap and
// lets the user fix it themselves.
//
// The gap is split in two, because the remedy differs and one of them is a trap:
//
// * `installable` groups name packages apt can pull from the configured sources,
//   so `sudo apt-get install -f` really does resolve them.
// * `unavailable` groups name packages that exist in *no* configured apt source
//   (e.g. `docker-ce-cli`, shipped only by Docker's own repository). Running
//   `apt-get install -f` there cannot install anything; apt degenerates to
//   *removing* the very package that is being installed. Telling the user to run
//   it anyway is what turns one failed install into an uninstall loop, so this
//   bucket needs to say "add the vendor's apt source" instead.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

const DPKG_DEB_BIN: &str = "/usr/bin/dpkg-deb";
const DPKG_QUERY_BIN: &str = "/usr/bin/dpkg-query";
const DPKG_BIN: &str = "/usr/bin/dpkg";
const APT_CACHE_BIN: &str = "/usr/bin/apt-cache";
const SAFE_SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";

/// `apt-cache policy` prefixes the candidate line with this. Two spellings: the
/// helper pins `LC_ALL=C`, but a user-facing terminal may be localized, and
/// missing the line would silently downgrade every gap to "installable".
const CANDIDATE_LABELS: [&str; 2] = ["Candidate:", "候选："];

/// `apt-cache policy` renders an absent version like this. Both spellings for
/// the same reason as [`CANDIDATE_LABELS`]; these are plain ASCII even in the
/// Chinese catalogue, which is Unicode-fullwidth.
const NO_CANDIDATE_MARKERS: [&str; 2] = ["(none)", "(无)"];

/// Advisory dependency gap for the plan UI, split by whether apt can fix it.
/// Groups keep their original control-file text (`libfoo (>= 1.2)`,
/// `python3 | python3.11`) so the user sees exactly what the vendor declared.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DependencyGap {
    pub installable: Vec<String>,
    pub unavailable: Vec<String>,
}

/// Names of the control fields that must be satisfied before `dpkg --install`
/// can succeed, in the order Debian processes them.
const DEPENDENCY_FIELDS: [&str; 2] = ["Pre-Depends", "Depends"];

struct Alternative {
    name: String,
    /// `(operator, version)` when the alternative carries a version constraint.
    constraint: Option<(String, String)>,
}

struct DependencyGroup {
    /// Original, trimmed text of the comma-separated group (e.g.
    /// `libgtk-3-0 (>= 3.24)` or `python3 | python3.11`). Shown verbatim to the user.
    raw: String,
    alternatives: Vec<Alternative>,
}

struct InstalledIndex {
    versions: HashMap<String, String>,
    provides: HashSet<String>,
}

/// Dependency groups of `deb_path` that the locally installed set does not
/// satisfy, split by whether the configured apt sources could complete them.
/// Never fails the caller: when the control file or `dpkg-query` cannot be read,
/// a single advisory entry lands in `installable` so the UI still warns the user
/// to double-check. Only a *successfully parsed* policy listing that shows no
/// candidate puts a group in `unavailable` — an unreadable apt state must never
/// claim a dependency is impossible to install.
pub(crate) fn dependency_gap(deb_path: &Path) -> DependencyGap {
    let control = match command_stdout(DPKG_DEB_BIN, &["-f", &deb_path.to_string_lossy()]) {
        Ok(text) => text,
        Err(_) => {
            return DependencyGap {
                installable: vec!["无法读取安装包依赖信息，安装前请手动检查".to_owned()],
                unavailable: Vec::new(),
            };
        }
    };
    let combined = dependency_fields(&control);
    if combined.trim().is_empty() {
        return DependencyGap::default();
    }
    let installed = match installed_index() {
        Some(index) => index,
        None => {
            return DependencyGap {
                installable: vec!["无法读取本机软件包状态，安装前请手动检查依赖".to_owned()],
                unavailable: Vec::new(),
            };
        }
    };
    let mut gap = DependencyGap::default();
    for group in parse_dependency_groups(&combined) {
        if group_satisfied(&group, &installed) {
            continue;
        }
        if group_installable(&group) {
            gap.installable.push(group.raw);
        } else {
            gap.unavailable.push(group.raw);
        }
    }
    gap
}

/// Whether any alternative of `group` has a candidate version in the configured
/// apt sources. A group with no parseable alternative is reported as installable
/// so we never over-claim that apt is unable to help.
fn group_installable(group: &DependencyGroup) -> bool {
    if group.alternatives.is_empty() {
        return true;
    }
    group
        .alternatives
        .iter()
        .any(|alternative| package_has_apt_candidate(&alternative.name))
}

fn package_has_apt_candidate(name: &str) -> bool {
    match command_stdout(APT_CACHE_BIN, &["policy", name]) {
        Ok(output) => package_has_apt_candidate_in(&output),
        // apt itself failed (no privileges to read the lists, a corrupt cache):
        // assume apt can help rather than sending the user after a missing source.
        Err(_) => true,
    }
}

/// `apt-cache policy <pkg>` on a package absent from every source still succeeds
/// and prints an empty version list with a `(none)` candidate:
///
/// ```text
/// docker-ce-cli:
///   已安装：(无)
///   候选： (无)
///   版本列表：
/// ```
///
/// Any other outcome (a real candidate, or output we cannot recognize) counts as
/// installable.
fn package_has_apt_candidate_in(output: &str) -> bool {
    let Some(rest) = output.lines().find_map(|line| {
        let line = line.trim();
        CANDIDATE_LABELS
            .iter()
            .find_map(|label| line.strip_prefix(label))
    }) else {
        return true;
    };
    let candidate = rest.trim();
    !NO_CANDIDATE_MARKERS.contains(&candidate)
}

/// Joins `Pre-Depends` and `Depends` from a `dpkg-deb -f` control dump, keeping
/// RFC-822 continuation lines.
fn dependency_fields(control: &str) -> String {
    let mut collected = Vec::new();
    let mut active = false;
    for raw_line in control.lines() {
        let trimmed = raw_line.trim_end();
        if let Some(stripped) = trimmed.strip_prefix(' ') {
            // Continuation line of the field we are currently collecting.
            if active {
                collected.push(stripped.trim_start());
            }
            continue;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if DEPENDENCY_FIELDS.contains(&name) {
                active = true;
                collected.push(value.trim_start());
                continue;
            }
        }
        active = false;
    }
    collected.join(" ")
}

fn command_stdout(program: &str, args: &[&str]) -> Result<String, String> {
    let output = clean_command(program)
        .args(args)
        .output()
        .map_err(|error| format!("无法执行 {program}：{error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn clean_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", SAFE_SYSTEM_PATH)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("LANGUAGE", "C");
    command
}

fn installed_index() -> Option<InstalledIndex> {
    let output = clean_command(DPKG_QUERY_BIN)
        .args([
            "-W",
            "-f",
            "${binary:Package}\t${Version}\t${Provides}\t${Status}\n",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(installed_index_from_lines(
        &String::from_utf8_lossy(&output.stdout),
    ))
}

fn installed_index_from_lines(input: &str) -> InstalledIndex {
    let mut versions = HashMap::new();
    let mut provides = HashSet::new();
    for line in input.lines() {
        let mut fields = line.splitn(4, '\t');
        let package = fields.next().unwrap_or("").split(':').next().unwrap_or("").trim();
        let version = fields.next().unwrap_or("").trim();
        let provides_field = fields.next().unwrap_or("");
        let status = fields.next().unwrap_or("").trim();
        if status != "install ok installed" || package.is_empty() || version.is_empty() {
            continue;
        }
        versions.insert(package.to_owned(), version.to_owned());
        for provide in provides_field.split(',') {
            let name = provide.split_whitespace().next().unwrap_or("").trim();
            if !name.is_empty() {
                provides.insert(name.to_owned());
            }
        }
    }
    InstalledIndex { versions, provides }
}

fn parse_dependency_groups(value: &str) -> Vec<DependencyGroup> {
    value
        .split(',')
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(|group| DependencyGroup {
            raw: group.to_owned(),
            alternatives: group
                .split('|')
                .filter_map(parse_alternative)
                .collect(),
        })
        .collect()
}

fn parse_alternative(token: &str) -> Option<Alternative> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let name_end = token.find(['(', ':']).unwrap_or(token.len());
    let name = token[..name_end].trim().to_owned();
    if name.is_empty() || !valid_package_token(&name) {
        return None;
    }
    Some(Alternative {
        name,
        constraint: parse_constraint(token),
    })
}

fn parse_constraint(token: &str) -> Option<(String, String)> {
    let start = token.find('(')?;
    let end = token.rfind(')')?;
    if end <= start {
        return None;
    }
    let inner = token[start + 1..end].trim();
    let op_len = if inner.starts_with("<<") || inner.starts_with("<=") || inner.starts_with(">=") || inner.starts_with(">>") {
        2
    } else if inner.starts_with('<') || inner.starts_with('>') || inner.starts_with('=') {
        1
    } else {
        return None;
    };
    let operator = inner[..op_len].trim();
    let version = inner[op_len..].trim();
    if operator.is_empty() || version.is_empty() {
        return None;
    }
    Some((operator.to_owned(), version.to_owned()))
}

fn valid_package_token(name: &str) -> bool {
    name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.'))
}

fn group_satisfied(group: &DependencyGroup, installed: &InstalledIndex) -> bool {
    if group.alternatives.is_empty() {
        // A dependency we failed to parse; do not warn the user about a syntax
        // quirk in a vendor control file — be permissive instead.
        return true;
    }
    group
        .alternatives
        .iter()
        .any(|alternative| alternative_satisfied(alternative, installed))
}

fn alternative_satisfied(alternative: &Alternative, installed: &InstalledIndex) -> bool {
    if let Some(version) = installed.versions.get(&alternative.name) {
        return match &alternative.constraint {
            Some((operator, required)) => version_satisfies(version, operator, required),
            None => true,
        };
    }
    if installed.provides.contains(&alternative.name) {
        // A virtual package: only accept it when the dependency carries no
        // version constraint — versioned `Provides` is rare and we would risk a
        // false negative by trusting it blindly.
        return alternative.constraint.is_none();
    }
    false
}

fn version_satisfies(installed: &str, operator: &str, required: &str) -> bool {
    clean_command(DPKG_BIN)
        .args(["--compare-versions", installed, operator, required])
        .status()
        .is_ok_and(|status| status.success())
}

// Batch updates deliberately stop instead of attempting dependency repair. This
// is a read-only veto, not an APT solver or a new software metadata source.
#[derive(Clone, Debug)]
struct BatchPackage {
    name: String,
    version: String,
    provides: String,
    depends: String,
    conflicts: String,
}

pub(crate) fn batch_preflight(plans: &[umanager_plan::OperationPlan]) -> Result<(), String> {
    let audit = command_stdout(DPKG_BIN, &["--audit"])?;
    if !audit.trim().is_empty() {
        return Err(format!("系统存在未完成的软件包操作，请先处理后再更新：{}", audit.chars().take(2000).collect::<String>()));
    }
    let output = command_stdout(DPKG_QUERY_BIN, &["-W", "-f", "${binary:Package}\t${Version}\t${Status}\t${Provides}\t${Depends}\t${Pre-Depends}\t${Conflicts}\t${Breaks}\n"])?;
    let installed = batch_packages(&output)?;
    let mut targets = Vec::new();
    for plan in plans {
        let text = command_stdout(DPKG_DEB_BIN, &["-f", &plan.payload.deb_path])?;
        let fields = control_fields(&text);
        if fields.get("Package") != Some(&plan.payload.package_name) || fields.get("Version") != Some(&plan.payload.target_version) {
            return Err("安装包元数据与批量计划不一致".into());
        }
        targets.push(BatchPackage {
            name: plan.payload.package_name.clone(), version: plan.payload.target_version.clone(),
            provides: fields.get("Provides").cloned().unwrap_or_default(),
            depends: [fields.get("Depends"), fields.get("Pre-Depends")].into_iter().flatten().cloned().collect::<Vec<_>>().join(", "),
            conflicts: [fields.get("Conflicts"), fields.get("Breaks")].into_iter().flatten().cloned().collect::<Vec<_>>().join(", "),
        });
    }
    check_batch_relations(&installed, &targets)
}

fn control_fields(control: &str) -> HashMap<String, String> {
    let mut result: HashMap<String, String> = HashMap::new();
    let mut previous: Option<String> = None;
    for line in control.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some(value) = previous.as_ref().and_then(|key| result.get_mut(key)) { value.push(' '); value.push_str(line.trim()); }
        } else if let Some((key, value)) = line.split_once(':') {
            previous = Some(key.into()); result.insert(key.into(), value.trim().into());
        } else { previous = None; }
    }
    result
}
fn batch_packages(output: &str) -> Result<Vec<BatchPackage>, String> {
    let mut result = Vec::new();
    for line in output.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 8 { return Err("无法完整读取本机软件包关系，已停止批量更新".into()); }
        if !fields[2].ends_with(" ok installed") { continue; }
        result.push(BatchPackage { name: fields[0].split(':').next().unwrap_or("").into(), version: fields[1].into(), provides: fields[3].into(), depends: [fields[4], fields[5]].into_iter().filter(|v| !v.is_empty()).collect::<Vec<_>>().join(", "), conflicts: [fields[6], fields[7]].into_iter().filter(|v| !v.is_empty()).collect::<Vec<_>>().join(", ") });
    }
    Ok(result)
}
fn relation_index(packages: &[BatchPackage]) -> InstalledIndex {
    InstalledIndex {
        versions: packages.iter().map(|p| (p.name.clone(), p.version.clone())).collect(),
        provides: packages.iter().flat_map(|p| p.provides.split(',').filter_map(parse_alternative).map(|a| a.name)).collect(),
    }
}
fn check_batch_relations(installed: &[BatchPackage], targets: &[BatchPackage]) -> Result<(), String> {
    let target_names: HashSet<_> = targets.iter().map(|p| &p.name).collect();
    let mut projected: Vec<_> = installed.iter().filter(|p| !target_names.contains(&p.name)).cloned().collect();
    projected.extend_from_slice(targets);
    // Check both the current and projected sets. That also catches conflicts
    // introduced only when two selected packages have both been upgraded.
    for target in targets {
        check_conflicts(target, installed)?;
    }
    for package in &projected {
        if !package.conflicts.is_empty() { check_conflicts(package, &projected)?; }
    }
    let before = relation_index(installed);
    let after = relation_index(&projected);
    for package in &projected {
        for group in parse_dependency_groups(&package.depends) {
            // Only veto newly broken reverse dependencies. Existing unrelated
            // dependency problems are not inferred to be caused by this batch.
            if group_satisfied(&group, &before) && !group_satisfied(&group, &after) {
                return Err(format!("更新会破坏 {} 的依赖：{}；请先处理后再批量更新", package.name, group.raw));
            }
        }
    }
    Ok(())
}
fn check_conflicts(package: &BatchPackage, others: &[BatchPackage]) -> Result<(), String> {
    if package.conflicts.is_empty() { return Ok(()); }
    let index = InstalledIndex {
        versions: others.iter().filter(|p| p.name != package.name).map(|p| (p.name.clone(), p.version.clone())).collect(),
        provides: others.iter().filter(|p| p.name != package.name).flat_map(|p| p.provides.split(',').filter_map(parse_alternative).map(|a| a.name)).collect(),
    };
    for group in parse_dependency_groups(&package.conflicts) {
        if group.alternatives.is_empty() { return Err(format!("无法解析 {} 的冲突约束：{}", package.name, group.raw)); }
        if group.alternatives.iter().any(|a| alternative_satisfied(a, &index) || (index.provides.contains(&a.name) && a.constraint.is_some())) {
            return Err(format!("{} 存在 Conflicts / Breaks 冲突：{}；请先处理后再批量更新", package.name, group.raw));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(lines: &str) -> InstalledIndex {
        installed_index_from_lines(lines)
    }

    fn parse_one(group: &str) -> DependencyGroup {
        parse_dependency_groups(group).pop().unwrap()
    }

    #[test]
    fn parses_alternatives_constraints_and_arch_qualifiers() {
        let groups = parse_dependency_groups("libgtk-3-0 (>= 3.24), python3 | python3.11, libc6:any");
        assert_eq!(groups.len(), 3);

        assert_eq!(groups[0].raw, "libgtk-3-0 (>= 3.24)");
        assert_eq!(groups[0].alternatives.len(), 1);
        assert_eq!(groups[0].alternatives[0].name, "libgtk-3-0");
        assert_eq!(
            groups[0].alternatives[0].constraint,
            Some((">=".to_owned(), "3.24".to_owned()))
        );

        assert_eq!(groups[1].raw, "python3 | python3.11");
        assert_eq!(groups[1].alternatives.len(), 2);
        assert_eq!(groups[1].alternatives[0].name, "python3");
        assert_eq!(groups[1].alternatives[1].name, "python3.11");

        // `libc6:any` drops the architecture qualifier.
        assert_eq!(groups[2].raw, "libc6:any");
        assert_eq!(groups[2].alternatives[0].name, "libc6");
        assert!(groups[2].alternatives[0].constraint.is_none());
    }

    #[test]
    fn group_is_satisfied_by_any_alternative() {
        let installed = index(
            "python3.11\t3.11.0-1\t\tinstall ok installed\n\
             libgtk-3-0\t3.24.43-1\t\tinstall ok installed\n",
        );
        assert!(group_satisfied(&parse_one("python3 | python3.11"), &installed));
        assert!(!group_satisfied(&parse_one("python3 | python4"), &installed));
    }

    #[test]
    fn virtual_package_without_constraint_is_satisfied_via_provides() {
        let installed = index(
            "adwaita-icon-theme\t50.0-1\tgnome-icon-theme-symbolic\tinstall ok installed\n",
        );
        assert!(group_satisfied(&parse_one("gnome-icon-theme-symbolic"), &installed));
        // Versioned virtual dependency is conservatively reported as missing.
        assert!(!group_satisfied(&parse_one("gnome-icon-theme-symbolic (>= 1)"), &installed));
    }

    #[test]
    fn constrained_dependency_checks_installed_version() {
        let installed = index("libgtk-3-0\t3.22.0-1\t\tinstall ok installed\n");
        // 3.22 < 3.24, so `>= 3.24` is unsatisfied.
        assert!(!group_satisfied(&parse_one("libgtk-3-0 (>= 3.24)"), &installed));
        assert!(group_satisfied(&parse_one("libgtk-3-0 (>= 3.0)"), &installed));
    }

    #[test]
    fn dependency_fields_keep_continuation_lines() {
        let control = "Package: x\nPre-Depends: liba (>= 1),\n libb\nDepends: libc\n";
        assert_eq!(dependency_fields(control), "liba (>= 1), libb libc");
    }

    // Verbatim `apt-cache policy docker-ce-cli` output from an Ubuntu 26.04 host
    // whose configured sources do not carry Docker's own repository.
    const POLICY_WITHOUT_CANDIDATE: &str =
        "docker-ce-cli:\n  已安装：(无)\n  候选： (无)\n  版本列表：\n";

    const POLICY_WITH_CANDIDATE: &str =
        "qemu-system-x86:\n  已安装：(无)\n  候选： 1:10.2.1+ds-1ubuntu3.2\n  版本列表：\n";

    #[test]
    fn policy_without_a_candidate_is_not_installable() {
        assert!(!package_has_apt_candidate_in(POLICY_WITHOUT_CANDIDATE));
        // The C-locale spelling, as seen when apt runs under `LC_ALL=C`.
        assert!(!package_has_apt_candidate_in(
            "docker-ce-cli:\n  Installed: (none)\n  Candidate: (none)\n  Version table:\n"
        ));
    }

    #[test]
    fn policy_with_a_candidate_is_installable() {
        assert!(package_has_apt_candidate_in(POLICY_WITH_CANDIDATE));
        assert!(package_has_apt_candidate_in(
            "libgtk-3-0:\n  Installed: 3.24.43-1\n  Candidate: 3.24.43-2\n  Version table:\n"
        ));
    }

    #[test]
    fn unrecognizable_policy_output_never_claims_unavailable() {
        // apt-cache failing loudly, or a future output format: stay permissive so
        // the user is never wrongly told a dependency cannot be installed.
        assert!(package_has_apt_candidate_in(""));
        assert!(package_has_apt_candidate_in(
            "E: The package lists or status file could not be parsed"
        ));
    }

    #[test]
    fn gap_serializes_into_two_buckets() {
        let gap = DependencyGap {
            installable: vec!["pass".to_owned()],
            unavailable: vec!["docker-ce-cli".to_owned()],
        };
        assert!(!gap.installable.is_empty() && !gap.unavailable.is_empty());
        // An empty gap must still serialize both buckets, so the UI can rely on
        // the shape instead of guarding every access.
        let empty = serde_json::to_value(DependencyGap::default()).unwrap();
        assert_eq!(empty["installable"].as_array().map(Vec::len), Some(0));
        assert_eq!(empty["unavailable"].as_array().map(Vec::len), Some(0));
        let json = serde_json::to_value(&gap).unwrap();
        assert_eq!(json["installable"][0], "pass");
        assert_eq!(json["unavailable"][0], "docker-ce-cli");
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    fn package(name: &str, version: &str, depends: &str, conflicts: &str, provides: &str) -> BatchPackage {
        BatchPackage { name: name.into(), version: version.into(), depends: depends.into(), conflicts: conflicts.into(), provides: provides.into() }
    }
    #[test]
    fn accepts_independent_updates_and_ignores_self_conflicts() {
        let current = vec![package("a", "1", "", "", "virtual-a")];
        let target = vec![package("a", "2", "", "a, virtual-a", "virtual-a")];
        assert!(check_batch_relations(&current, &target).is_ok());
    }
    #[test]
    fn detects_a_conflict_introduced_only_by_two_target_versions() {
        let current = vec![package("a", "1", "", "", ""), package("b", "1", "", "", "")];
        let target = vec![package("a", "2", "", "b (>= 2)", ""), package("b", "2", "", "", "")];
        assert!(check_batch_relations(&current, &target).unwrap_err().contains("Conflicts"));
    }
    #[test]
    fn detects_current_and_reverse_conflicts_and_virtual_packages() {
        let current = vec![package("old", "1", "", "new (>= 2)", "virtual-conflict")];
        assert!(check_batch_relations(&current, &[package("new", "2", "", "", "")]).is_err());
        assert!(check_batch_relations(&current, &[package("new", "2", "", "virtual-conflict", "")]).is_err());
        assert!(check_batch_relations(&current, &[package("new", "2", "", "virtual-conflict (>= 2)", "")]).is_err());
    }
    #[test]
    fn detects_newly_broken_reverse_dependency() {
        let current = vec![package("app", "1", "library (<< 2)", "", ""), package("library", "1", "", "", "")];
        let targets = vec![package("library", "2", "", "", "")];
        assert!(check_batch_relations(&current, &targets).unwrap_err().contains("app"));
    }
    #[test]
    fn permits_an_alternative_dependency_that_remains_satisfied() {
        let current = vec![package("app", "1", "library (<< 2) | fallback", "", ""), package("library", "1", "", "", ""), package("fallback", "1", "", "", "")];
        assert!(check_batch_relations(&current, &[package("library", "2", "", "", "")]).is_ok());
    }
    #[test]
    fn replacing_a_provider_does_not_leave_old_virtual_provides() {
        let current = vec![package("app", "1", "old-virtual", "", ""), package("provider", "1", "", "", "old-virtual")];
        assert!(check_batch_relations(&current, &[package("provider", "2", "", "", "new-virtual")]).is_err());
    }
    #[test]
    fn malformed_conflict_constraints_fail_closed() {
        assert!(check_batch_relations(&[], &[package("app", "2", "", "???", "")]).is_err());
    }
    #[test]
    fn reads_multiline_control_fields_and_breaks() {
        let fields = control_fields("Package: app\nVersion: 2\nDepends: library,\n other\nBreaks: legacy (<< 2)\n");
        assert_eq!(fields["Depends"], "library, other"); assert_eq!(fields["Breaks"], "legacy (<< 2)");
    }
    #[test]
    fn parses_installed_and_held_packages_but_rejects_partial_output() {
        let rows = "a:amd64\t1\tinstall ok installed\tvirtual-a\tlibrary\t\told\tlegacy\nb\t2\thold ok installed\t\t\t\t\t\nc\t1\tdeinstall ok config-files\t\t\t\t\t\n";
        let result = batch_packages(rows).unwrap(); assert_eq!(result.len(), 2); assert_eq!(result[0].name, "a"); assert_eq!(result[0].conflicts, "old, legacy");
        assert!(batch_packages("incomplete\t1").is_err());
    }
}
