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
