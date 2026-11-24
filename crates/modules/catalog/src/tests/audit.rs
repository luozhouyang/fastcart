use super::*;

#[tokio::test]
async fn audited_mutations_append_outbox_rows_once() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    let service = CatalogService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let product = basic_product(&service, "Audited", "AUD-1").await;
    insert_sales_channel(&db, "sc_audit").await;

    service.archive_product(&product.id).await.unwrap();
    // Archiving an already archived product is a no-op.
    service.archive_product(&product.id).await.unwrap();
    service
        .set_product_sales_channels(&product.id, &["sc_audit".to_owned()])
        .await
        .unwrap();
    // Replacing the same channel set is a no-op.
    service
        .set_product_sales_channels(&product.id, &["sc_audit".to_owned()])
        .await
        .unwrap();
    service.delete_product(&product.id).await.unwrap();
    // A repeat delete fails and must not append another row.
    assert!(service.delete_product(&product.id).await.is_err());

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec![
            "product.archived",
            "product.sales_channels_updated",
            "product.deleted"
        ]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );
}
