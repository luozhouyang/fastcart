use super::*;
use crate::{ResolvedTranslation, UpsertTranslationInput, resolve_translation};

fn translation_input(
    resource_type: &str,
    resource_id: &str,
    locale: &str,
    fields: serde_json::Value,
) -> UpsertTranslationInput {
    UpsertTranslationInput {
        resource_type: resource_type.to_owned(),
        resource_id: resource_id.to_owned(),
        locale: locale.to_owned(),
        fields,
    }
}

#[tokio::test]
async fn upserts_and_lists_translations() {
    let service = service().await;

    let created = service
        .upsert_translation(translation_input(
            "product",
            "prd_1",
            "fr_FR",
            json!({"title": "Chaise"}),
        ))
        .await
        .unwrap();
    assert_eq!(created.locale, "fr-FR", "locales are normalized");
    assert_eq!(created.resource_type, "product");
    assert_eq!(created.fields["title"], "Chaise");
    assert!(created.id.starts_with("tr_"));

    let updated = service
        .upsert_translation(translation_input(
            "product",
            "prd_1",
            "fr-FR",
            json!({"title": "Chaise longue", "description": "Bois"}),
        ))
        .await
        .unwrap();
    assert_eq!(updated.id, created.id, "a second upsert replaces the row");
    assert_eq!(updated.fields["title"], "Chaise longue");
    assert_eq!(updated.created_at, created.created_at);
    assert!(updated.updated_at >= created.updated_at);

    service
        .upsert_translation(translation_input(
            "product",
            "prd_1",
            "de-DE",
            json!({"title": "Stuhl"}),
        ))
        .await
        .unwrap();
    service
        .upsert_translation(translation_input(
            "category",
            "cat_1",
            "fr-FR",
            json!({"title": "Meubles"}),
        ))
        .await
        .unwrap();

    let rows = service.list_translations("product", "prd_1").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].locale, "de-DE");
    assert_eq!(rows[1].locale, "fr-FR");

    let category_rows = service
        .list_translations("category", "cat_1")
        .await
        .unwrap();
    assert_eq!(category_rows.len(), 1);

    let fetched = service
        .get_translation("product", "prd_1", "fr-FR")
        .await
        .unwrap();
    assert_eq!(fetched, updated);
}

#[tokio::test]
async fn upsert_validates_its_input() {
    let service = service().await;

    let error = service
        .upsert_translation(translation_input(
            "order",
            "ord_1",
            "fr-FR",
            json!({"title": "x"}),
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .upsert_translation(translation_input(
            "product",
            "prd_1",
            "not a locale",
            json!({"title": "x"}),
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .upsert_translation(translation_input("product", "prd_1", "fr-FR", json!(["x"])))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .upsert_translation(translation_input(
            "product",
            "  ",
            "fr-FR",
            json!({"title": "x"}),
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let huge = json!({"title": "x".repeat(crate::MAX_TRANSLATION_BYTES)});
    let error = service
        .upsert_translation(translation_input("product", "prd_1", "fr-FR", huge))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    assert!(
        service
            .list_translations("product", "prd_1")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn deletes_translations_and_reports_missing_rows() {
    let service = service().await;
    service
        .upsert_translation(translation_input(
            "page",
            "page_1",
            "fr-FR",
            json!({"title": "Accueil"}),
        ))
        .await
        .unwrap();

    service
        .delete_translation("page", "page_1", "fr-FR")
        .await
        .unwrap();
    assert!(
        service
            .list_translations("page", "page_1")
            .await
            .unwrap()
            .is_empty()
    );

    let error = service
        .delete_translation("page", "page_1", "fr-FR")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));

    let error = service
        .get_translation("page", "page_1", "fr-FR")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
}

#[tokio::test]
async fn resolves_requested_locale_then_default_fallback() {
    let service = service().await;
    for (locale, fields) in [
        ("fr-FR", json!({"title": "Chaise"})),
        ("en-US", json!({"title": "Chair", "description": "A chair"})),
    ] {
        service
            .upsert_translation(translation_input("product", "prd_1", locale, fields))
            .await
            .unwrap();
    }
    let rows = service.list_translations("product", "prd_1").await.unwrap();

    let resolved: ResolvedTranslation =
        resolve_translation(&rows, "fr-CA", &["fr-FR".to_owned(), "en-US".to_owned()]);
    assert_eq!(resolved.fields["title"], "Chaise");
    assert_eq!(resolved.fields["description"], "A chair");
    assert_eq!(resolved.source_of("title"), Some("fr-FR"));
    assert_eq!(resolved.source_of("description"), Some("en-US"));
    assert_eq!(resolved.source_of("handle"), None);

    // The service read path applies the same merge and normalizes the locale.
    let served = service
        .resolve_translations(
            "product",
            "prd_1",
            "fr_ca",
            &["fr-FR".to_owned(), "en-US".to_owned()],
        )
        .await
        .unwrap();
    assert_eq!(served, resolved);

    let error = service
        .resolve_translations("order", "ord_1", "fr-FR", &[])
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn translation_writes_are_audited() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("usr_1".to_owned()));

    service
        .upsert_translation(translation_input(
            "product",
            "prd_1",
            "fr-FR",
            json!({"title": "Chaise"}),
        ))
        .await
        .unwrap();
    service
        .delete_translation("product", "prd_1", "fr-FR")
        .await
        .unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    let actions: Vec<&str> = audits.iter().map(|audit| audit.action.as_str()).collect();
    assert!(actions.contains(&"translation.upserted"));
    assert!(actions.contains(&"translation.deleted"));
}
