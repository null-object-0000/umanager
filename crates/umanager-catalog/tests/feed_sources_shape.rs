//! Guard for the CI-only `feed-sources.json` extra-app catalog.
//!
//! `scripts/update-feed.mjs` copies every entry there **verbatim** into the
//! signed `catalogJson` the desktop app merges and the privileged helper
//! authorizes, so a typo'd field name or a misspelled `source.kind` is only
//! visible in production: the app would fail to merge the new software (and its
//! install/update path) long after the feed was published. Assert here that each
//! entry deserializes into the shared `Application` schema.
use umanager_catalog::Application;

#[test]
fn every_feed_source_application_matches_the_catalog_schema() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../feed-sources.json");
    let text = std::fs::read_to_string(&path).expect("read feed-sources.json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("parse feed-sources.json");
    let apps = json["applications"].as_array().expect("applications array");
    assert!(!apps.is_empty());
    for app in apps {
        let id = app["applicationId"].as_str().unwrap_or("<missing applicationId>");
        let parsed: Application = serde_json::from_value(app.clone())
            .unwrap_or_else(|error| panic!("feed-sources.json 的 {id} 不符合 catalog schema：{error}"));
        assert_eq!(parsed.application_id, id);
    }
    println!("validated {} feed-sources applications", apps.len());
}

#[test]
fn antigravity_ide_uses_its_product_version_and_preserves_user_archive_safety() {
    let json: serde_json::Value = serde_json::from_str(include_str!("../../../feed-sources.json")).unwrap();
    let app = json["applications"].as_array().unwrap().iter().find(|app| app["applicationId"] == "antigravity-ide").unwrap();
    assert_eq!(app["source"]["kind"], "tarGzV2");
    let parsed: Application = serde_json::from_value(app.clone()).unwrap();
    assert!(parsed.is_auto_installable() && parsed.is_user_archive());
    assert_eq!(parsed.download_hosts(), ["edgedl.me.gvt1.com"]);
    if let umanager_catalog::SourceSpec::TarGz { version_path, version_field, .. } = parsed.source {
        assert_eq!(version_field.as_deref(), Some("ideVersion"));
        assert_eq!(version_path.as_deref(), Some("Antigravity IDE/resources/app/product.json"));
    } else { panic!("IDE must use a user archive"); }
}
