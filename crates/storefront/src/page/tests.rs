use serde_json::json;

use super::{MAX_PAGE_BLOCKS, PageBody, validate_binding_value};
use crate::editor::ThemeEditorManifest;

fn editor() -> ThemeEditorManifest {
    ThemeEditorManifest::parse(&json!({
        "schema_version": 1,
        "categories": {
            "hero": { "title": "Hero", "components": ["Hero"] },
            "product": { "title": "Product", "components": ["ProductGrid"] }
        },
        "components": {
            "Hero": {
                "title": "Hero",
                "category": "hero",
                "fields": {
                    "title": { "type": "text", "required": true },
                    "subtitle": { "type": "text" }
                },
                "defaultProps": { "title": "Welcome" },
                "slots": [{ "name": "content" }]
            },
            "ProductGrid": {
                "title": "Product Grid",
                "category": "product",
                "fields": {
                    "collection": {
                        "type": "data_source",
                        "source": "collection",
                        "required": true
                    },
                    "limit": { "type": "number", "min": 1, "max": 24, "default": 4 }
                },
                "defaultProps": { "limit": 4 },
                "slots": []
            }
        }
    }))
    .unwrap()
}

fn body() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "content": [
            { "type": "Hero", "props": { "id": "hero-1", "title": "Welcome" } },
            {
                "type": "ProductGrid",
                "props": {
                    "id": "grid-1",
                    "collection": { "id": "col_1", "handle": "featured", "label": "Featured" },
                    "limit": 8
                }
            }
        ],
        "root": { "props": { "title": "Home" } },
        "zones": {}
    })
}

#[test]
fn parses_empty_and_valid_bodies() {
    assert!(PageBody::parse(&json!(null)).unwrap().content.is_empty());
    assert!(PageBody::parse(&json!({})).unwrap().content.is_empty());
    PageBody::parse(&body()).unwrap();
}

#[test]
fn rejects_unknown_blocks_and_props() {
    let manifest = editor();
    let mut value = body();
    value["content"][0]["type"] = json!("Missing");
    let parsed = PageBody::parse(&value).unwrap();
    assert!(parsed.validate_with_editor(&manifest).is_err());

    let mut value = body();
    value["content"][0]["props"]["unknown"] = json!("x");
    let parsed = PageBody::parse(&value).unwrap();
    assert!(parsed.validate_with_editor(&manifest).is_err());

    let mut value = body();
    value["content"][0]["props"]["title"] = json!(42);
    let parsed = PageBody::parse(&value).unwrap();
    assert!(parsed.validate_with_editor(&manifest).is_err());
}

#[test]
fn duplicate_block_ids_are_rejected() {
    let mut value = body();
    value["content"][1]["props"]["id"] = json!("hero-1");
    assert!(PageBody::parse(&value).is_err());
}

#[test]
fn reserved_context_block_id_is_rejected() {
    let mut value = body();
    value["content"][0]["props"]["id"] = json!("context");
    let error = PageBody::parse(&value).expect_err("context is reserved");
    assert!(error.to_string().contains("reserved"));
}

#[test]
fn collecting_after_validation_matches_full_collection() {
    let manifest = editor();
    let parsed = PageBody::parse(&body()).unwrap();
    let validated = parsed.collect_bindings(&manifest).unwrap();
    let after_validation = parsed.collect_bindings_after_validation(&manifest).unwrap();
    assert_eq!(validated, after_validation);
}

#[test]
fn block_count_is_bounded() {
    let mut value = body();
    let content = (0..=MAX_PAGE_BLOCKS)
        .map(|index| {
            json!({
                "type": "Hero",
                "props": { "id": format!("hero-{index}"), "title": "x" }
            })
        })
        .collect::<Vec<_>>();
    value["content"] = serde_json::Value::Array(content);
    assert!(PageBody::parse(&value).is_err());
}

#[test]
fn collects_typed_bindings() {
    let manifest = editor();
    let parsed = PageBody::parse(&body()).unwrap();
    let bindings = parsed.collect_bindings(&manifest).unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].block_id, "grid-1");
    assert_eq!(bindings[0].field, "collection");
    assert!(matches!(
        bindings[0].binding,
        crate::data::DataBinding::Collection { limit: Some(8), .. }
    ));
}

#[test]
fn product_list_and_search_binding_shapes_are_strict() {
    assert!(
        validate_binding_value(
            "product_list",
            "products",
            &json!({"collection_id": "col_1"})
        )
        .is_ok()
    );
    assert!(
        validate_binding_value("product_list", "products", &json!({"collection_id": 123})).is_err()
    );
    assert!(validate_binding_value("product_list", "products", &json!({"id": "col_1"})).is_err());
    assert!(validate_binding_value("product_list", "products", &json!({"sort": "price"})).is_err());
    assert!(
        validate_binding_value(
            "product_list",
            "products",
            &json!({"sort": "title_desc", "limit": 4})
        )
        .is_ok()
    );
    assert!(validate_binding_value("search", "products", &json!({"query": "tee"})).is_ok());
    assert!(
        validate_binding_value(
            "search",
            "products",
            &json!({"query": "tee", "sort": "asc"})
        )
        .is_err()
    );
}

#[test]
fn inquiry_form_binding_shape_is_strict() {
    assert!(
        validate_binding_value(
            "inquiry_form",
            "form",
            &json!({"handle": "contact", "label": "Contact"})
        )
        .is_ok()
    );
    assert!(validate_binding_value("inquiry_form", "form", &json!({"handle": "contact"})).is_ok());
    assert!(validate_binding_value("inquiry_form", "form", &json!({"id": "iform_1"})).is_err());
    assert!(validate_binding_value("inquiry_form", "form", &json!({"handle": "Contact"})).is_err());
    assert!(
        validate_binding_value("inquiry_form", "form", &json!({"handle": "contact-us"})).is_ok()
    );
    assert!(
        validate_binding_value(
            "inquiry_form",
            "form",
            &json!({"handle": "contact", "notification_fields": []})
        )
        .is_err()
    );
}
