use std::collections::BTreeMap;

use fastcart_plugin::manifest::{PluginManifest, validate_settings};
use fastcart_plugin::{ContributionRegistry, PluginError};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "schema_version": "1",
        "id": "chat-widget",
        "name": "Live Chat",
        "version": "1.2.0",
        "api_version": 2,
        "runtime": {
            "engine": "workerd",
            "compatibility_date": "2026-11-01",
            "compatibility_flags": ["nodejs_compat"]
        },
        "entry": { "type": "worker", "path": "worker/index.js" },
        "capabilities": {
            "data": ["catalog.read", "orders.read"],
            "storage": ["kv.read", "kv.write"],
            "secrets": ["secrets.read"],
            "events": ["order.placed"],
            "network": ["api.example.com"],
            "admin": ["admin.ui"],
            "storefront": ["store.read"]
        },
        "extensions": {
            "admin": [
                { "target": "admin.nav.section", "label": "Live Chat", "route": "/apps/chat" },
                { "target": "admin.page.settings.tabs", "title": "Chat", "entry": "ui/settings.html" }
            ],
            "storefront": [
                { "kind": "embed", "placement": "body:end", "entry": "storefront/boot.js", "pages": ["*"] }
            ]
        },
        "settings": {
            "schema": {
                "type": "object",
                "properties": { "accent": { "type": "string" } },
                "additionalProperties": false
            }
        },
        "rpc": ["conversations.list", "conversations.reply"],
        "rpc_permissions": { "conversations.reply": "orders.write" },
        "jobs": [{ "name": "sync", "schedule": "*/5 * * * *" }]
    })
}

fn parse(value: &Value) -> Result<PluginManifest, PluginError> {
    PluginManifest::parse(value.to_string().as_bytes())
}

fn expect_invalid(value: &Value) {
    let manifest = parse(value).expect("manifest must parse");
    assert!(
        manifest.validate().is_err(),
        "manifest should be invalid: {value}"
    );
}

#[test]
fn accepts_a_valid_manifest() {
    let manifest = parse(&base()).expect("parse");
    manifest.validate().expect("valid");
    let scopes: Vec<&str> = manifest
        .scopes()
        .unwrap()
        .iter()
        .map(|s| s.as_str())
        .collect();
    for expected in [
        "catalog.read",
        "kv.write",
        "secrets.read",
        "events.subscribe",
        "network.egress",
        "admin.ui",
        "storefront.ui",
    ] {
        assert!(scopes.contains(&expected), "missing {expected}");
    }
}

#[test]
fn rejects_bad_identity_and_version() {
    let mut value = base();
    value["id"] = json!("Bad Id");
    expect_invalid(&value);

    let mut value = base();
    value["version"] = json!("1.2");
    expect_invalid(&value);

    let mut value = base();
    value["api_version"] = json!(1);
    expect_invalid(&value);
}

#[test]
fn rejects_unknown_and_reserved_targets() {
    let mut value = base();
    value["extensions"]["admin"][1]["target"] = json!("admin.nope.render");
    expect_invalid(&value);

    let mut value = base();
    value["extensions"]["admin"][1]["target"] = json!("admin.page.orders.details.actions");
    expect_invalid(&value);
}

#[test]
fn rejects_write_scopes() {
    let mut value = base();
    value["capabilities"]["data"] = json!(["catalog.write"]);
    expect_invalid(&value);
}

#[test]
fn rejects_secret_write_and_unknown_secret_scopes() {
    for scope in ["secrets.write", "secrets.read.all"] {
        let mut value = base();
        value["capabilities"]["secrets"] = json!([scope]);
        expect_invalid(&value);
    }

    let mut value = base();
    value["capabilities"]["secrets"] = json!(["kv.read"]);
    expect_invalid(&value);
}

#[test]
fn rejects_bad_egress_hosts() {
    for host in [
        "https://api.example.com",
        "10.0.0.1",
        "API.example.com",
        "api.example.com:443",
    ] {
        let mut value = base();
        value["capabilities"]["network"] = json!([host]);
        expect_invalid(&value);
    }
}

#[test]
fn rejects_bad_schedules_and_duplicate_jobs() {
    let mut value = base();
    value["jobs"] = json!([{ "name": "sync", "schedule": "every five minutes" }]);
    expect_invalid(&value);

    let mut value = base();
    value["jobs"] = json!([{ "name": "sync" }, { "name": "sync" }]);
    expect_invalid(&value);
}

#[test]
fn rejects_a_non_object_settings_schema() {
    let mut value = base();
    value["settings"]["schema"] = json!({ "type": "string" });
    expect_invalid(&value);
}

#[test]
fn validates_settings_against_the_schema() {
    let manifest = parse(&base()).unwrap();
    assert!(validate_settings(&manifest, &json!({ "accent": "#fff" })).is_ok());
    assert!(validate_settings(&manifest, &json!({ "accent": 1 })).is_err());
    assert!(validate_settings(&manifest, &json!({ "unknown": true })).is_err());
}

#[test]
fn the_published_example_is_valid() {
    let example = include_str!("../../../schemas/plugin-manifest/examples/chat-widget.json");
    let manifest = PluginManifest::parse(example.as_bytes()).expect("example parses");
    manifest.validate().expect("example validates");

    let schema = include_str!("../../../schemas/plugin-manifest/manifest.schema.json");
    let schema: Value = serde_json::from_str(schema).expect("schema parses");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let instance: Value = serde_json::from_str(example).expect("example parses as JSON");
    assert!(
        validator.is_valid(&instance),
        "example violates the JSON schema: {:?}",
        validator.iter_errors(&instance).next()
    );
}

#[test]
fn rejects_invalid_or_duplicate_rpc_methods() {
    let mut value = base();
    value["rpc"] = json!(["_private"]);
    expect_invalid(&value);

    let mut value = base();
    value["rpc"] = json!(["a", "a"]);
    expect_invalid(&value);
}

#[test]
fn rejects_rpc_permissions_for_undeclared_or_malformed_methods() {
    let mut value = base();
    value["rpc_permissions"] = json!({ "conversations.archive": "orders.write" });
    expect_invalid(&value);

    let mut value = base();
    value["rpc_permissions"] = json!({ "conversations.reply": "Orders.Write" });
    expect_invalid(&value);

    let mut value = base();
    value["rpc_permissions"] = json!({ "conversations.reply": "orders" });
    expect_invalid(&value);
}

#[test]
fn the_chat_template_manifest_is_valid() {
    let template = include_str!("../../../templates/plugin-chat/plugin.json");
    let manifest = PluginManifest::parse(template.as_bytes()).expect("template parses");
    manifest.validate().expect("template validates");
    assert!(manifest.extensions.admin.len() == 3);
    assert!(manifest.extensions.storefront.len() == 1);
    assert!(manifest.rpc.contains(&"conversations.list".to_owned()));
}

fn capability_contribution() -> Value {
    json!({
        "kind": "storefront_capability",
        "id": "search",
        "version": 1,
        "title": "Search",
        "config_schema": {
            "type": "object",
            "properties": { "placeholder": { "type": "string" } }
        }
    })
}

fn component_contribution(entry: &str) -> Value {
    json!({
        "kind": "storefront_component",
        "id": "hero-banner",
        "title": "Hero banner",
        "category": "marketing",
        "editor_manifest": { "fields": ["title", "image"] },
        "entry": entry,
        "requires": [
            { "id": "search", "version": 1 },
            { "id": "recommendations", "optional": true }
        ]
    })
}

fn archive_files(paths: &[&str]) -> BTreeMap<String, u64> {
    paths.iter().map(|path| ((*path).to_owned(), 1)).collect()
}

fn contribution_manifest() -> Value {
    let mut value = base();
    value["capabilities"]["storefront"] = json!(["store.read", "storefront.component"]);
    value["provides"] = json!([
        capability_contribution(),
        component_contribution("components/hero.js")
    ]);
    value
}

#[test]
fn accepts_valid_storefront_contributions() {
    let manifest = parse(&contribution_manifest()).expect("parse");
    manifest.validate().expect("valid");
    assert_eq!(manifest.provides.len(), 2);
    assert_eq!(manifest.provides[0].kind(), "storefront_capability");
    assert_eq!(manifest.provides[1].kind(), "storefront_component");

    let scopes: Vec<&str> = manifest
        .scopes()
        .unwrap()
        .iter()
        .map(|scope| scope.as_str())
        .collect();
    assert!(
        scopes.contains(&"storefront.component"),
        "component scope missing: {scopes:?}"
    );

    let files = archive_files(&[
        "plugin.json",
        "worker/index.js",
        "ui/settings.html",
        "storefront/boot.js",
        "components/hero.js",
    ]);
    manifest
        .validate_archive(&files, 1024 * 1024)
        .expect("all referenced files exist");

    let schema: Value = serde_json::from_str(include_str!(
        "../../../schemas/plugin-manifest/manifest.schema.json"
    ))
    .expect("schema parses");
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let instance = contribution_manifest();
    assert!(
        validator.is_valid(&instance),
        "contributions violate the JSON schema: {:?}",
        validator.iter_errors(&instance).next()
    );
}

#[test]
fn omits_the_component_scope_unless_requested() {
    let manifest = parse(&base()).expect("parse");
    let scopes: Vec<&str> = manifest
        .scopes()
        .unwrap()
        .iter()
        .map(|scope| scope.as_str())
        .collect();
    assert!(!scopes.contains(&"storefront.component"));
}

#[test]
fn rejects_duplicate_contributions_and_requirements() {
    let mut value = contribution_manifest();
    value["provides"]
        .as_array_mut()
        .unwrap()
        .push(capability_contribution());
    expect_invalid(&value);

    let mut value = contribution_manifest();
    value["provides"]
        .as_array_mut()
        .unwrap()
        .push(component_contribution("components/other.js"));
    expect_invalid(&value);

    let mut value = contribution_manifest();
    value["provides"][1]["requires"] =
        json!([{ "id": "search" }, { "id": "search", "version": 2 }]);
    expect_invalid(&value);
}

#[test]
fn rejects_bad_contribution_ids_and_versions() {
    let long_id = "a".repeat(121);
    for id in ["Bad Id", "-leading", "trailing-", long_id.as_str()] {
        let mut value = contribution_manifest();
        value["provides"][0]["id"] = json!(id);
        expect_invalid(&value);
    }

    let mut value = contribution_manifest();
    value["provides"][1]["id"] = json!("UPPER");
    expect_invalid(&value);

    let mut value = contribution_manifest();
    value["provides"][0]["version"] = json!(0);
    expect_invalid(&value);

    let mut value = contribution_manifest();
    value["provides"][1]["requires"][0]["version"] = json!(0);
    expect_invalid(&value);

    let mut value = contribution_manifest();
    value["provides"][1]["requires"][0]["id"] = json!("Bad/Id");
    expect_invalid(&value);
}

#[test]
fn rejects_bad_contribution_schemas_and_manifests() {
    let mut value = contribution_manifest();
    value["provides"][0]["config_schema"] = json!("not-an-object");
    expect_invalid(&value);

    let mut value = contribution_manifest();
    value["provides"][0]["config_schema"] = json!({ "type": "nonsense" });
    expect_invalid(&value);

    let mut value = contribution_manifest();
    value["provides"][1]["editor_manifest"] = json!([1, 2, 3]);
    expect_invalid(&value);
}

#[test]
fn rejects_unsafe_component_entries() {
    for entry in [
        "../hero.js",
        "/components/hero.js",
        "components/hero.css",
        "",
    ] {
        let mut value = contribution_manifest();
        value["provides"][1]["entry"] = json!(entry);
        expect_invalid(&value);
    }
}

#[test]
fn rejects_a_missing_component_entry_file() {
    let manifest = parse(&contribution_manifest()).expect("parse");
    manifest.validate().expect("valid");
    let files = archive_files(&[
        "plugin.json",
        "worker/index.js",
        "ui/settings.html",
        "storefront/boot.js",
    ]);
    let error = manifest
        .validate_archive(&files, 1024 * 1024)
        .expect_err("missing entry must be rejected");
    assert!(
        matches!(error, PluginError::InvalidManifest(_)),
        "{error:?}"
    );
}

#[test]
fn rejects_components_without_the_storefront_component_scope() {
    let mut value = contribution_manifest();
    value["capabilities"]["storefront"] = json!(["store.read"]);
    expect_invalid(&value);
}

#[test]
fn rejects_more_than_twenty_contributions() {
    let mut value = contribution_manifest();
    value["provides"] = Value::Array(
        (0..21)
            .map(|index| {
                json!({
                    "kind": "storefront_capability",
                    "id": format!("capability-{index}"),
                    "version": 1,
                    "title": "Capability"
                })
            })
            .collect(),
    );
    expect_invalid(&value);
}

#[test]
fn the_registry_detects_duplicates_across_plugins() {
    let mut first = contribution_manifest();
    first["id"] = json!("plugin-a");
    let mut second = contribution_manifest();
    second["id"] = json!("plugin-b");
    second["provides"] = json!([capability_contribution()]);

    let first = parse(&first).expect("first parses");
    first.validate().expect("first validates");
    let second = parse(&second).expect("second parses");
    second.validate().expect("second validates");

    let registry = ContributionRegistry::from_plugins([
        ("plugin-b".to_owned(), second),
        ("plugin-a".to_owned(), first),
    ]);
    let capabilities = registry.capabilities();
    assert_eq!(capabilities.len(), 2);
    assert_eq!(capabilities[0].plugin_id, "plugin-a");
    assert_eq!(capabilities[1].plugin_id, "plugin-b");
    assert_eq!(registry.components().len(), 1);
    assert_eq!(registry.components()[0].plugin_id, "plugin-a");
    assert_eq!(registry.find_capability("search", Some(1)).len(), 2);
    assert!(registry.find_capability("search", Some(2)).is_empty());

    let duplicates = registry.duplicates();
    assert_eq!(duplicates.len(), 1);
    assert_eq!(duplicates[0].len(), 2);
    assert_eq!(
        duplicates[0][0].contribution.kind(),
        "storefront_capability"
    );
}
