use super::*;

#[tokio::test]
async fn mutations_append_audit_entries_in_the_same_transaction() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let page = service.create_page(page_input("Home")).await.unwrap();
    service.publish_page(&page.id).await.unwrap();
    service.archive_page(&page.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(
        audits
            .iter()
            .map(|audit| audit.action.as_str())
            .collect::<Vec<_>>(),
        vec!["page.created", "page.published", "page.archived"]
    );
    assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
    assert!(
        audits
            .iter()
            .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
    );
}
