use std::future::Future;

const GITHUB_HOST: &str = "null-object-0000.github.io";
const MIRROR_HOST: &str = "updates.snewbie.site";

pub(crate) struct Endpoint {
    pub url: String,
    pub hosts: Vec<String>,
    label: &'static str,
}

/// Transport aliases only. Signed source URLs, endorsements and cache keys remain
/// canonical GitHub identities. Custom feeds never inherit our mirror allowlist.
pub(crate) fn endpoints(url: &str, hosts: &[String]) -> Vec<Endpoint> {
    let mut result = Vec::new();
    if let Ok(mut parsed) = url::Url::parse(url)
        && parsed.scheme() == "https"
        && parsed.host_str() == Some(GITHUB_HOST)
        && parsed.port_or_known_default() == Some(443)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.path().starts_with("/umanager/")
        && !parsed.path().contains('%')
        && hosts.iter().any(|host| host == GITHUB_HOST)
    {
        parsed
            .set_host(Some(MIRROR_HOST))
            .expect("fixed mirror host");
        result.push(Endpoint {
            url: parsed.to_string(),
            hosts: vec![MIRROR_HOST.to_owned()],
            label: "EdgeOne 镜像",
        });
    }
    result.push(Endpoint {
        url: url.to_owned(),
        hosts: hosts.to_vec(),
        label: "原始元数据源",
    });
    result
}

/// A candidate only succeeds after its caller has verified the complete feed /
/// signature pair (or icon digest). Never combine data from different endpoints.
pub(crate) async fn verified_fallback<T, F, Fut>(
    candidates: Vec<Endpoint>,
    mut fetch_verified: F,
) -> Result<T, String>
where
    F: FnMut(Endpoint) -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let mut failures = Vec::new();
    for candidate in candidates {
        let label = candidate.label;
        match fetch_verified(candidate).await {
            Ok(value) => return Ok(value),
            Err(error) => failures.push(format!("{label}：{error}")),
        }
    }
    Err(failures.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_identity_is_preserved_and_hosts_are_isolated() {
        for path in ["feed.json", "v3/feed.tencent.json", "v3/icons/qq.png"] {
            let canonical = format!("https://{GITHUB_HOST}/umanager/{path}");
            let list = endpoints(&canonical, &[GITHUB_HOST.to_owned()]);
            assert_eq!(list.len(), 2);
            assert_eq!(
                list[0].url,
                format!("https://{MIRROR_HOST}/umanager/{path}")
            );
            assert_eq!(list[0].hosts, [MIRROR_HOST]);
            assert_eq!(list[1].url, canonical);
            assert_eq!(list[1].hosts, [GITHUB_HOST]);
        }
    }

    #[test]
    fn mirror_cannot_bypass_an_original_host_restriction() {
        let url = format!("https://{GITHUB_HOST}/umanager/feed.json");
        let list = endpoints(&url, &["other.example".to_owned()]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].hosts, ["other.example"]);
    }

    #[test]
    fn unrelated_and_ambiguous_urls_do_not_use_the_mirror() {
        for url in [
            "http://null-object-0000.github.io/umanager/feed.json",
            "https://null-object-0000.github.io:8443/umanager/feed.json",
            "https://null-object-0000.github.io/another/feed.json",
            "https://null-object-0000.github.io/umanager-evil/feed.json",
            "https://null-object-0000.github.io/umanager/../other/feed.json",
            "https://null-object-0000.github.io/umanager/%2ffeed.json",
            "https://null-object-0000.github.io/umanager/feed.json?token=private",
            "https://null-object-0000.github.io/umanager/feed.json#fragment",
            "https://user:password@null-object-0000.github.io/umanager/feed.json",
            "https://null-object-0000.github.io.evil.test/umanager/feed.json",
            "https://custom.example/feed.json",
        ] {
            let list = endpoints(url, &[GITHUB_HOST.to_owned()]);
            assert_eq!(list.len(), 1, "unexpected alias for {url}");
            assert_eq!(list[0].url, url);
        }
    }

    #[test]
    fn failed_verification_retries_the_entire_canonical_candidate() {
        tauri::async_runtime::block_on(async {
            let mut attempted = Vec::new();
            let list = endpoints(
                "https://null-object-0000.github.io/umanager/v3/feed.json",
                &[GITHUB_HOST.to_owned()],
            );
            let value = verified_fallback(list, |endpoint| {
                attempted.push(endpoint.url.clone());
                async move {
                    if endpoint.hosts == [MIRROR_HOST] {
                        Err("元数据源签名校验失败".to_owned())
                    } else {
                        Ok("verified canonical pair")
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(value, "verified canonical pair");
            assert_eq!(attempted.len(), 2);
        });
    }

    #[test]
    fn no_unverified_result_is_returned_when_both_candidates_fail() {
        tauri::async_runtime::block_on(async {
            let list = endpoints(
                "https://null-object-0000.github.io/umanager/feed.json",
                &[GITHUB_HOST.to_owned()],
            );
            let result: Result<(), String> = verified_fallback(list, |_| async {
                Err("invalid signature or digest".to_owned())
            })
            .await;
            assert!(result.is_err());
            let error = result.unwrap_err();
            assert!(error.contains("EdgeOne 镜像"));
            assert!(error.contains("原始元数据源"));
        });
    }

    #[test]
    fn verified_mirror_success_does_not_contact_the_fallback() {
        tauri::async_runtime::block_on(async {
            let list = endpoints(
                "https://null-object-0000.github.io/umanager/feed.json",
                &[GITHUB_HOST.to_owned()],
            );
            let mut calls = 0;
            let result = verified_fallback(list, |_| {
                calls += 1;
                async { Ok::<_, String>("verified mirror pair") }
            })
            .await;
            assert_eq!(result.unwrap(), "verified mirror pair");
            assert_eq!(calls, 1);
        });
    }
}
