//! Integration-style tests for the theme registry service.

use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{AppError, Pagination, StoreId, ThemeId};
use fastcart_storefront::{ThemeEditorRef, ThemeManifest};
use serde_json::{Value, json};

use crate::{
    RegisterThemeInput, ThemeCapabilityBindingInput, ThemeService, ThemeSettingsRevisionStatus,
    UpdateThemeSettingsInput,
};

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
        routes: Default::default(),
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
        assets_manifest: None,
        settings: None,
        editor_manifest,
    }
}

fn input(id: &str) -> RegisterThemeInput {
    RegisterThemeInput {
        manifest: manifest(id),
        bundle_path: format!("themes/{id}"),
        settings_schema: Some(schema()),
        assets_manifest: None,
        settings: Some(json!({ "logo": "/logo.svg" })),
        editor_manifest: None,
    }
}

fn input_with_version(id: &str, version: &str) -> RegisterThemeInput {
    let mut input = input(id);
    input.manifest.version = version.to_owned();
    input
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
async fn mutations_append_audit_entries_atomically() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ThemeService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let first = service.register(input("first")).await.unwrap();
    let second = service.register(input("second")).await.unwrap();
    let third = service.register(input("third")).await.unwrap();
    service
        .update_settings(
            &first.id,
            UpdateThemeSettingsInput {
                settings: json!({ "logo": "/new.svg" }),
            },
        )
        .await
        .unwrap();
    service.activate(&first.id).await.unwrap();
    // Activating the already-active theme is a no-op and must not be audited.
    service.activate(&first.id).await.unwrap();
    service.activate(&second.id).await.unwrap();
    service.rollback().await.unwrap();
    // `second` is now the rollback target (deleting it would be a conflict),
    // so delete the unrelated inactive theme instead.
    service.delete(&third.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "theme.registered",
            "theme.registered",
            "theme.registered",
            "theme.settings_updated",
            "theme.activated",
            "theme.activated",
            "theme.rolled_back",
            "theme.deleted",
        ]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );

    // Deleting the active theme is rejected and must not append an audit row.
    assert!(service.delete(&first.id).await.is_err());
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 8);
}

#[tokio::test]
async fn rejects_duplicate_manifest_versions_but_allows_new_versions() {
    let service = service().await;
    service.register(input("default")).await.unwrap();

    let error = service.register(input("default")).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let second = service
        .register(input_with_version("default", "2.0.0"))
        .await
        .unwrap();
    assert_eq!(second.manifest.id, "default");
    assert_eq!(second.manifest.version, "2.0.0");

    let listed = service.list(pagination(10, 0)).await.unwrap();
    assert_eq!(listed.count, 2);
}

#[tokio::test]
async fn stores_assets_manifests_and_settings_revisions() {
    let service = service().await;
    let mut input = input("default");
    input.assets_manifest = Some(json!({ "main.js": "dist/main.js" }));

    let theme = service.register(input).await.unwrap();
    assert_eq!(
        theme.assets_manifest,
        Some(json!({ "main.js": "dist/main.js" }))
    );
    assert_eq!(theme.settings_revision, 1);
    assert_eq!(theme.draft_settings, None);

    let revisions = service.settings_revisions(&theme.id, 10).await.unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].revision, 1);
    assert_eq!(revisions[0].status, ThemeSettingsRevisionStatus::Published);
    assert_eq!(revisions[0].settings, json!({ "logo": "/logo.svg" }));
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
async fn settings_drafts_publish_and_restore_revisions() {
    let service = service().await;
    let theme = service.register(input("default")).await.unwrap();

    let draft = service
        .save_settings_draft(
            &theme.id,
            json!({ "logo": "/draft.svg", "brand_color": "#abc" }),
            Some("user_1".to_owned()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(draft.settings, json!({ "logo": "/logo.svg" }));
    assert_eq!(
        draft.draft_settings,
        Some(json!({ "logo": "/draft.svg", "brand_color": "#abc" }))
    );
    assert_eq!(draft.settings_revision, 1);

    let revisions = service.settings_revisions(&theme.id, 10).await.unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].revision, 2);
    assert_eq!(revisions[0].status, ThemeSettingsRevisionStatus::Draft);
    assert_eq!(revisions[0].created_by.as_deref(), Some("user_1"));

    // Saving again replaces the working draft instead of adding a revision.
    let draft = service
        .save_settings_draft(
            &theme.id,
            json!({ "logo": "/draft-2.svg" }),
            Some("user_2".to_owned()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        draft.draft_settings,
        Some(json!({ "logo": "/draft-2.svg" }))
    );
    assert_eq!(draft.settings_revision, 1);
    assert_eq!(
        service
            .settings_revisions(&theme.id, 10)
            .await
            .unwrap()
            .len(),
        2
    );

    let published = service
        .publish_settings(&theme.id, Some("user_2".to_owned()), None)
        .await
        .unwrap();
    assert_eq!(published.settings, json!({ "logo": "/draft-2.svg" }));
    assert_eq!(published.settings_revision, 2);
    assert_eq!(published.draft_settings, None);
    let revisions = service.settings_revisions(&theme.id, 10).await.unwrap();
    assert_eq!(revisions[0].revision, 2);
    assert_eq!(revisions[0].status, ThemeSettingsRevisionStatus::Published);

    // A stale concurrency token is rejected without touching the draft.
    let error = service
        .save_settings_draft(&theme.id, json!({ "logo": "/stale.svg" }), None, Some(1))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ConflictDetails { .. }));

    // Restoring revision 1 appends a new published revision.
    let restored = service
        .restore_settings_revision(&theme.id, 1, Some("user_3".to_owned()), None)
        .await
        .unwrap();
    assert_eq!(restored.settings, json!({ "logo": "/logo.svg" }));
    assert_eq!(restored.settings_revision, 3);
    assert_eq!(restored.draft_settings, None);
    let revisions = service.settings_revisions(&theme.id, 10).await.unwrap();
    assert_eq!(revisions[0].revision, 3);
    assert_eq!(revisions[0].status, ThemeSettingsRevisionStatus::Published);
    assert_eq!(revisions[0].created_by.as_deref(), Some("user_3"));
    assert_eq!(revisions[1].revision, 2);

    // Invalid drafts never reach the live settings or the revision history.
    let error = service
        .save_settings_draft(&theme.id, json!({ "brand_color": "#000" }), None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .restore_settings_revision(&theme.id, 99, None, None)
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn capability_bindings_support_create_update_and_delete() {
    let service = service().await;
    let theme = service.register(input("default")).await.unwrap();

    assert!(
        service
            .list_capability_bindings(&theme.id)
            .await
            .unwrap()
            .is_empty()
    );

    let binding = service
        .upsert_capability_binding(
            &theme.id,
            ThemeCapabilityBindingInput {
                capability_id: "chat".to_owned(),
                endpoint: "https://plugins.example.com/chat".to_owned(),
                config: json!({ "region": "eu" }),
                is_active: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(binding.capability_id, "chat");
    assert_eq!(binding.config, json!({ "region": "eu" }));
    assert!(binding.is_active);

    let updated = service
        .upsert_capability_binding(
            &theme.id,
            ThemeCapabilityBindingInput {
                capability_id: "chat".to_owned(),
                endpoint: "https://plugins.example.com/chat/v2".to_owned(),
                config: json!({}),
                is_active: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.id, binding.id);
    assert_eq!(updated.endpoint, "https://plugins.example.com/chat/v2");
    assert!(!updated.is_active);
    assert_eq!(updated.created_at, binding.created_at);

    assert_eq!(
        service
            .list_capability_bindings(&theme.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let error = service
        .upsert_capability_binding(
            &theme.id,
            ThemeCapabilityBindingInput {
                capability_id: "search".to_owned(),
                endpoint: "plugins/search".to_owned(),
                config: json!({}),
                is_active: true,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .upsert_capability_binding(
            &theme.id,
            ThemeCapabilityBindingInput {
                capability_id: "search".to_owned(),
                endpoint: "https://plugins.example.com/search".to_owned(),
                config: json!(["not", "an", "object"]),
                is_active: true,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    service
        .delete_capability_binding(&theme.id, "chat")
        .await
        .unwrap();
    assert!(
        service
            .list_capability_bindings(&theme.id)
            .await
            .unwrap()
            .is_empty()
    );

    let error = service
        .delete_capability_binding(&theme.id, "chat")
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

    let history = service.activation_history(10).await.unwrap();
    assert_eq!(history.len(), 4);
    assert_eq!(history[0].theme_id, second.id);
    assert_eq!(history[0].manifest_id.as_deref(), Some("second"));
    assert_eq!(history[0].previous_theme_id.as_ref(), Some(&first.id));
    assert_eq!(history[1].theme_id, first.id);
    assert_eq!(history[2].theme_id, second.id);
    assert_eq!(history[2].previous_theme_id.as_ref(), Some(&first.id));
    assert_eq!(history[3].theme_id, first.id);
    assert!(history[3].previous_theme_id.is_none());
}

#[tokio::test]
async fn deletion_is_rejected_while_rollback_targets_the_theme() {
    let service = service().await;
    let first = service.register(input("first")).await.unwrap();
    let second = service.register(input("second")).await.unwrap();
    service.activate(&first.id).await.unwrap();
    service.activate(&second.id).await.unwrap();

    // The latest activation points rollback at `first`, so deleting it would
    // make a later rollback fail.
    let error = service.delete(&first.id).await.unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let rolled_back = service.rollback().await.unwrap();
    assert_eq!(rolled_back.id, first.id);
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
