use crate::source_engine::{CANCELED_ERROR, CancelCheck, DownloadPlan};
use std::{future::Future, path::Path, time::Duration};
use umanager_catalog::Application;

pub(crate) struct Endpoint {
    pub url: String,
    pub hosts: Vec<String>,
    pub timeout: Duration,
}

/// An exact transport alias for our own signed release only. The immutable
/// plan, helper authorization and canonical package-host whitelist stay intact.
pub(crate) fn endpoints(app: &Application, plan: &DownloadPlan) -> Vec<Endpoint> {
    let hosts: Vec<String> = app
        .download_hosts()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut result = Vec::new();
    let version = &plan.version;
    let asset = format!("UManager_{version}_amd64.deb");
    let canonical = format!(
        "https://github.com/null-object-0000/umanager/releases/download/v{version}/{asset}"
    );
    if app.application_id == "umanager"
        && plan.application_id == "umanager"
        && app.package_name == "u-manager"
        && plan.package_name == "u-manager"
        && app.architecture == "amd64"
        && plan.architecture == "amd64"
        && plan.archive.is_none()
        && valid_version(version)
        && plan.download_url == canonical
        && hosts.iter().any(|h| h == "github.com")
        && plan.expected_size > 0
        && plan
            .expected_sha256
            .as_deref()
            .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        result.push(Endpoint {
            url: format!("https://updates.snewbie.site/umanager/releases/v{version}/{asset}"),
            hosts: vec!["updates.snewbie.site".into()],
            timeout: Duration::from_secs(60),
        });
    }
    result.push(Endpoint {
        url: plan.download_url.clone(),
        hosts,
        timeout: Duration::from_secs(30 * 60),
    });
    result
}

fn valid_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Only a fully verified package can win. Remove any failed partial download
/// before starting the next candidate; cancellation must never start fallback.
pub(crate) async fn verified_fallback<T, F, Fut>(
    candidates: Vec<Endpoint>,
    path: &Path,
    cancel: &CancelCheck,
    mut fetch: F,
) -> Result<T, String>
where
    F: FnMut(Endpoint) -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let mut failures = Vec::new();
    for endpoint in candidates {
        if cancel() {
            return Err(CANCELED_ERROR.into());
        }
        match fetch(endpoint).await {
            Ok(value) if !cancel() => return Ok(value),
            result => {
                match tokio::fs::remove_file(path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(format!("无法清理失败下载：{error}")),
                }
                if cancel() {
                    return Err(CANCELED_ERROR.into());
                }
                if let Err(error) = result {
                    if error == CANCELED_ERROR {
                        return Err(error);
                    }
                    failures.push(error);
                }
            }
        }
    }
    Err(failures.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct TestDirectory(std::path::PathBuf);
    impl TestDirectory {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            assert!(
                self.0
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("umanager-package-test-")
            );
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn test_directory() -> TestDirectory {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "umanager-package-test-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&path).unwrap();
        TestDirectory(path)
    }
    fn fixture() -> (Application, DownloadPlan) {
        let app = umanager_catalog::Catalog::load()
            .unwrap()
            .self_update_source()
            .unwrap()
            .to_application();
        let plan = DownloadPlan {
            archive: None, application_id: "umanager".into(), package_name: "u-manager".into(),
            version: "0.18.5".into(), architecture: "amd64".into(),
            source_kind: crate::scanner::SourceKind::OfficialWebsite, repository_url: None,
            download_url: "https://github.com/null-object-0000/umanager/releases/download/v0.18.5/UManager_0.18.5_amd64.deb".into(),
            file_name: "unused.deb".into(), expected_size: 10, expected_sha256: Some("a".repeat(64)),
            target_path: "unused".into(), release_tag: Some("v0.18.5".into()), asset_name: Some("UManager_0.18.5_amd64.deb".into()), website_version: None,
        };
        (app, plan)
    }
    #[test]
    fn package_transport_preserves_canonical_plan_and_isolates_hosts() {
        let (app, plan) = fixture();
        let list = endpoints(&app, &plan);
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0].url,
            "https://updates.snewbie.site/umanager/releases/v0.18.5/UManager_0.18.5_amd64.deb"
        );
        assert_eq!(list[0].hosts, ["updates.snewbie.site"]);
        assert_eq!(list[1].url, plan.download_url);
        assert_eq!(list[1].hosts, app.download_hosts());
        assert!(!app.download_hosts().contains(&"updates.snewbie.site"));
    }
    #[test]
    fn package_transport_rejects_other_apps_hosts_paths_and_ambiguous_urls() {
        let (mut app, plan) = fixture();
        for url in [
            plan.download_url.replace("https:", "http:"),
            plan.download_url.replace("github.com/", "github.com:8443/"),
            plan.download_url
                .replace("github.com/", "user:pass@github.com/"),
            plan.download_url
                .replace("github.com/", "github.com.evil.test/"),
            plan.download_url.replace("/umanager/", "/other/"),
            plan.download_url.replace("v0.18.5/", "v0.18.4/"),
            format!("{}?token=secret", plan.download_url),
            format!("{}#fragment", plan.download_url),
            plan.download_url.replace("UManager_", "%55Manager_"),
        ] {
            let mut changed = plan.clone();
            changed.download_url = url;
            assert_eq!(endpoints(&app, &changed).len(), 1);
        }
        let mut changed = plan.clone();
        changed.application_id = "other".into();
        assert_eq!(endpoints(&app, &changed).len(), 1);
        app.application_id = "other".into();
        assert_eq!(endpoints(&app, &plan).len(), 1);
        let (mut app, _) = fixture();
        if let umanager_catalog::SourceSpec::ReleaseApi {
            asset_download_hosts,
            ..
        } = &mut app.source
        {
            asset_download_hosts.retain(|h| h != "github.com");
        }
        assert_eq!(endpoints(&app, &plan).len(), 1);
    }
    #[test]
    fn package_transport_failed_attempt_is_removed_before_github_fallback() {
        tauri::async_runtime::block_on(async {
            for failure in [
                "HTTP 404",
                "timeout",
                "wrong size",
                "wrong SHA256",
                "wrong deb metadata",
            ] {
                let (app, plan) = fixture();
                let directory = test_directory();
                let file = directory.path().join("download.tmp");
                let cancel: CancelCheck = Arc::new(|| false);
                let mut calls = 0;
                let result = verified_fallback(endpoints(&app, &plan), &file, &cancel, |_| {
                    calls += 1;
                    assert!(
                        !file.exists(),
                        "failed bytes must be discarded before retry"
                    );
                    let first = calls == 1;
                    let file = file.clone();
                    async move {
                        tokio::fs::write(
                            file,
                            if first {
                                b"bad".as_slice()
                            } else {
                                b"good".as_slice()
                            },
                        )
                        .await
                        .unwrap();
                        if first {
                            Err(failure.to_owned())
                        } else {
                            Ok("fully verified GitHub package")
                        }
                    }
                })
                .await
                .unwrap();
                assert_eq!(calls, 2);
                assert_eq!(result, "fully verified GitHub package");
                assert_eq!(std::fs::read(file).unwrap(), b"good");
            }
        });
    }
    #[test]
    fn package_transport_cancellation_does_not_fall_back_or_keep_partial_file() {
        tauri::async_runtime::block_on(async {
            let (app, plan) = fixture();
            let directory = test_directory();
            let file = directory.path().join("download.tmp");
            let stopped = Arc::new(AtomicBool::new(false));
            let flag = stopped.clone();
            let cancel: CancelCheck = Arc::new(move || flag.load(Ordering::SeqCst));
            let mut calls = 0;
            let result: Result<(), String> =
                verified_fallback(endpoints(&app, &plan), &file, &cancel, |_| {
                    calls += 1;
                    let stopped = stopped.clone();
                    let file = file.clone();
                    async move {
                        tokio::fs::write(file, b"partial").await.unwrap();
                        stopped.store(true, Ordering::SeqCst);
                        Err("read failed".into())
                    }
                })
                .await;
            assert_eq!(result.unwrap_err(), CANCELED_ERROR);
            assert_eq!(calls, 1);
            assert!(!file.exists());
        });
    }
    #[test]
    fn package_transport_verified_mirror_success_avoids_github() {
        tauri::async_runtime::block_on(async {
            let (app, plan) = fixture();
            let directory = test_directory();
            let file = directory.path().join("download.tmp");
            let cancel: CancelCheck = Arc::new(|| false);
            let mut calls = 0;
            let result = verified_fallback(endpoints(&app, &plan), &file, &cancel, |_| {
                calls += 1;
                async { Ok::<_, String>("fully verified mirror package") }
            })
            .await
            .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(result, "fully verified mirror package");
        });
    }
    #[test]
    fn package_transport_never_accepts_two_failed_candidates() {
        tauri::async_runtime::block_on(async {
            let (app, plan) = fixture();
            let directory = test_directory();
            let file = directory.path().join("download.tmp");
            let cancel: CancelCheck = Arc::new(|| false);
            let result: Result<(), String> =
                verified_fallback(endpoints(&app, &plan), &file, &cancel, |_| async {
                    Err("invalid package".into())
                })
                .await;
            assert!(result.is_err());
            assert!(!file.exists());
        });
    }
}
