use fastcart_ssr_workerd::ArtifactManifest;

#[test]
fn the_checked_in_example_matches_the_rust_manifest() {
    let raw = include_str!("../../../schemas/storefront-artifact/examples/tanstack-basic.json");
    let manifest: ArtifactManifest = serde_json::from_str(raw).expect("parse example");
    manifest.validate().expect("validate example");
    assert_eq!(manifest.schema_version, "1");
    assert_eq!(manifest.entry.path, "worker/index.js");
    assert_eq!(manifest.assets.base_path, "/_fastcart/assets/");
}

#[test]
fn the_json_schema_and_rust_required_fields_agree() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../schemas/storefront-artifact/manifest.schema.json"
    ))
    .expect("parse schema");
    let required = schema["required"].as_array().expect("required list");
    let raw = include_str!("../../../schemas/storefront-artifact/examples/tanstack-basic.json");
    let example: serde_json::Value = serde_json::from_str(raw).expect("parse example");
    for field in required {
        let field = field.as_str().expect("required field string");
        assert!(
            example.get(field).is_some(),
            "example is missing required field {field}"
        );
    }
}
