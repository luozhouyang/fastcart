//! Integration-style tests for the theme registry service.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Pagination, StoreId, ThemeId};
use fastcart_storefront::{ThemeEditorRef, ThemeManifest};
use serde_json::{Value, json};

use crate::{RegisterThemeInput, ThemeService, UpdateThemeSettingsInput};

async fn service() -> ThemeService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    ThemeService::new(db)
}

fn pagination(limit: i64, offset: i64) -> Pagination {
    Pagination::new(Some(limit), Some(offset)).unwrap()
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "logo": { "type": "string" },
            "brand_color": { "type": "string" }
        },
        "required": ["logo"]
    })
}

fn manifest(id: &str) -> ThemeManifest {
    ThemeManifest {
        id: id.to_owned(),
        name: format!("Theme {id}"),
        version: "1.0.0".to_owned(),
        runtime: "v8".to_owned(),
        entry: "dist/server.js".to_owned(),
        settings_schema: Some("settings.schema.json".to_owned()),
        assets_manifest: Some("assets.json".to_owned()),
        editor: None,
        requires: None,
    }
}

fn editor_ref() -> ThemeEditorRef {
    ThemeEditorRef {
        entry: "dist/editor.js".to_owned(),
        manifest: "editor.manifest.json".to_owned(),
        schema_version: 1,
    }
}

fn editor_manifest_value() -> Value {
    json!({
        "schema_version": 1,
        "categories": {
            "hero": { "title": "Hero", "components": ["Hero"] }
        },
        "components": {
            "Hero": {
                "title": "Hero",
                "category": "hero",
                "fields": {
                    "title": { "type": "text", "required": true }
                },
                "defaultProps": { "title": "Welcome" },
                "slots": []
            }
        },
        "root": { "fields": { "title": { "type": "text" } } }
    })
}

fn editor_input(id: &str, editor_manifest: Option<Value>) -> RegisterThemeInput {
    let mut declared = manifest(id);
    declared.editor = Some(editor_ref());
    RegisterThemeInput {
        manifest: declared,
        bundle_path: format!("themes/{id}"),
        settings_schema: None,
        settings: None,
        editor_manifest,
    }
}

fn input(id: &str) -> RegisterThemeInput {
    RegisterThemeInput {
        manifest: manifest(id),
        bundle_path: format!("themes/{id}"),
        settings_schema: Some(schema()),
        settings: Some(json!({ "logo": "/logo.svg" })),
        editor_manifest: None,
    }
}

#[tokio::test]
async fn registers_lists_and_reads_themes() {
    let service = service().await;

    let theme = service.register(input("default")).await.unwrap();
    assert_eq!(theme.manifest.id, "default");
    assert_eq!(theme.manifest.name, "Theme default");
    assert_eq!(theme.bundle_path, "themes/default");
    assert_eq!(theme.settings_schema, Some(schema()));
    assert_eq!(theme.settings, json!({ "logo": "/logo.svg" }));
    assert!(!theme.is_active);
    assert_eq!(theme.created_at, theme.updated_at);

    let fetched = service.get(&theme.id).await.unwrap();
    assert_eq!(fetched, theme);

    let listed = service.list(pagination(10, 0)).await.unwrap();
    assert_eq!(listed.count, 1);
    assert_eq!(listed.data[0].id, theme.id);

    let second = service.register(input("alternate")).await.unwrap();
    let listed = service.list(pagination(10, 0)).await.unwrap();
    assert_eq!(listed.count, 2);
    assert!(listed.data.iter().any(|theme| theme.id == second.id));

    assert!(service.active().await.unwrap().is_none());
}

#[tokio::test]
async fn rejects_duplicate_manifest_ids() {
    let service = service().await;
    service.register(input("default")).await.unwrap();

    let error = service.register(input("default")).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn validates_manifest_bundle_path_schema_and_settings() {
    let service = service().await;

    let error = service
        .register(RegisterThemeInput {
            manifest: ThemeManifest {
                id: "Bad Theme".to_owned(),
                ..manifest("bad")
            },
            ..input("bad")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .register(RegisterThemeInput {
            bundle_path: "../outside".to_owned(),
            ..input("escape")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .register(RegisterThemeInput {
            settings_schema: Some(json!(["not", "an", "object"])),
            ..input("array-schema")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .register(RegisterThemeInput {
            settings_schema: Some(json!({ "type": 5 })),
            ..input("malformed-schema")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .register(RegisterThemeInput {
            settings: Some(json!({ "brand_color": "#fff" })),
            ..input("missing-required")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let defaults = service
        .register(RegisterThemeInput {
            settings_schema: None,
            settings: None,
            ..input("defaults")
        })
        .await
        .unwrap();
    assert_eq!(defaults.settings, json!({}));
    assert!(defaults.settings_schema.is_none());
}

#[tokio::test]
async fn updates_settings_only_when_schema_valid() {
    let service = service().await;
    let theme = service.register(input("default")).await.unwrap();

    let error = service
        .update_settings(
            &theme.id,
            UpdateThemeSettingsInput {
                settings: json!({ "brand_color": "#000" }),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let updated = service
        .update_settings(
            &theme.id,
            UpdateThemeSettingsInput {
                settings: json!({ "logo": "/new.svg", "brand_color": "#000" }),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        updated.settings,
        json!({ "logo": "/new.svg", "brand_color": "#000" })
    );

    let error = service
        .update_settings(
            &ThemeId::new(),
            UpdateThemeSettingsInput {
                settings: json!({}),
            },
        )
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn activation_is_idempotent_and_keeps_one_active_theme() {
    let service = service().await;
    let first = service.register(input("first")).await.unwrap();
    let second = service.register(input("second")).await.unwrap();

    let error = service.activate(&ThemeId::new()).await.unwrap_err();
    assert!(error.is_not_found());

    let active = service.activate(&first.id).await.unwrap();
    assert!(active.is_active);

    let repeated = service.activate(&first.id).await.unwrap();
    assert!(repeated.is_active);
    assert_eq!(repeated.updated_at, active.updated_at);

    let other = service.activate(&second.id).await.unwrap();
    assert!(other.is_active);

    let listed = service.list(pagination(10, 0)).await.unwrap();
    let active: Vec<_> = listed.data.iter().filter(|theme| theme.is_active).collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, second.id);

    let current = service.active().await.unwrap().unwrap();
    assert_eq!(current.id, second.id);
}

#[tokio::test]
async fn rollback_returns_to_the_previous_theme() {
    let service = service().await;

    let error = service.rollback().await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let first = service.register(input("first")).await.unwrap();
    let second = service.register(input("second")).await.unwrap();
    service.activate(&first.id).await.unwrap();
    service.activate(&second.id).await.unwrap();

    let rolled_back = service.rollback().await.unwrap();
    assert_eq!(rolled_back.id, first.id);
    assert!(rolled_back.is_active);
    assert_eq!(service.active().await.unwrap().unwrap().id, first.id);

    let forward = service.rollback().await.unwrap();
    assert_eq!(forward.id, second.id);
    assert!(forward.is_active);
}

#[tokio::test]
async fn rollback_conflicts_when_the_previous_theme_is_gone() {
    let service = service().await;
    let first = service.register(input("first")).await.unwrap();
    let second = service.register(input("second")).await.unwrap();
    service.activate(&first.id).await.unwrap();
    service.activate(&second.id).await.unwrap();

    service.delete(&first.id).await.unwrap();

    let error = service.rollback().await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn deletes_inactive_themes_and_refuses_the_active_one() {
    let service = service().await;
    let first = service.register(input("first")).await.unwrap();
    let second = service.register(input("second")).await.unwrap();
    service.activate(&first.id).await.unwrap();

    let error = service.delete(&first.id).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    service.delete(&second.id).await.unwrap();
    assert!(service.get(&second.id).await.unwrap_err().is_not_found());

    let error = service.delete(&ThemeId::new()).await.unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn registers_and_returns_editor_manifests() {
    let service = service().await;
    let editor = editor_manifest_value();

    let theme = service
        .register(editor_input("editor", Some(editor.clone())))
        .await
        .unwrap();
    assert_eq!(theme.editor_manifest, Some(editor));
    assert_eq!(theme.schema_version, 1);

    let fetched = service.get(&theme.id).await.unwrap();
    assert_eq!(fetched.editor_manifest, theme.editor_manifest);
    assert_eq!(fetched.schema_version, 1);

    let listed = service.list(pagination(10, 0)).await.unwrap();
    assert_eq!(listed.data[0].editor_manifest, theme.editor_manifest);

    let parsed = service.editor_manifest(&theme.id).await.unwrap();
    assert_eq!(parsed.schema_version, 1);
    assert!(parsed.components.contains_key("Hero"));
}

#[tokio::test]
async fn themes_without_an_editor_default_to_schema_version_one() {
    let service = service().await;
    let theme = service.register(input("plain")).await.unwrap();

    assert!(theme.editor_manifest.is_none());
    assert_eq!(theme.schema_version, 1);

    let error = service.editor_manifest(&theme.id).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn rejects_editor_manifests_that_do_not_match_the_theme() {
    let service = service().await;

    let error = service
        .register(editor_input("missing", None))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .register(editor_input(
            "invalid",
            Some(json!({ "schema_version": 1, "components": {} })),
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .register(RegisterThemeInput {
            editor_manifest: Some(editor_manifest_value()),
            ..input("orphan")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn exposes_the_active_editor_manifest() {
    let service = service().await;
    assert!(service.active_editor_manifest().await.unwrap().is_none());

    let plain = service.register(input("plain")).await.unwrap();
    service.activate(&plain.id).await.unwrap();
    assert!(service.active_editor_manifest().await.unwrap().is_none());

    let themed = service
        .register(editor_input("editor", Some(editor_manifest_value())))
        .await
        .unwrap();
    service.activate(&themed.id).await.unwrap();

    let active = service.active_editor_manifest().await.unwrap().unwrap();
    assert_eq!(active.schema_version, 1);
    assert!(active.components.contains_key("Hero"));
}

#[tokio::test]
async fn missing_themes_return_not_found_errors() {
    let service = service().await;
    let missing = ThemeId::new();

    assert!(service.get(&missing).await.unwrap_err().is_not_found());
    assert!(service.activate(&missing).await.unwrap_err().is_not_found());
    assert!(service.delete(&missing).await.unwrap_err().is_not_found());
    assert!(
        service
            .editor_manifest(&missing)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .update_settings(
                &missing,
                UpdateThemeSettingsInput {
                    settings: json!({}),
                },
            )
            .await
            .unwrap_err()
            .is_not_found()
    );
}
