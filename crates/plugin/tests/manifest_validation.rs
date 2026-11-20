use fastcart_plugin::PluginError;
use fastcart_plugin::manifest::{PluginManifest, validate_settings};
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
fn the_chat_template_manifest_is_valid() {
    let template = include_str!("../../../templates/plugin-chat/plugin.json");
    let manifest = PluginManifest::parse(template.as_bytes()).expect("template parses");
    manifest.validate().expect("template validates");
    assert!(manifest.extensions.admin.len() == 3);
    assert!(manifest.extensions.storefront.len() == 1);
    assert!(manifest.rpc.contains(&"conversations.list".to_owned()));
}
