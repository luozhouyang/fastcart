use super::support::*;

#[tokio::test]
async fn confirming_an_edit_publishes_one_event_per_edit() {
    let fixture = fixture().await;
    let order = order(&fixture, 2, 1_000).await;
    commit_stock(&fixture, &order, 2).await;

    let edit = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 3))
        .await
        .unwrap();
    let confirmed = fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(confirmed.order_edit.difference_total, Some(1_000));

    let events = fastcart_infrastructure::fetch_pending_outbox(fixture.db.pool(), 100)
        .await
        .unwrap();
    let matching: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "order.edit_confirmed")
        .collect();
    assert_eq!(matching.len(), 1);
    let payload: serde_json::Value = serde_json::from_str(&matching[0].payload).unwrap();
    assert_eq!(payload["order_id"], order.id.as_str());
    assert_eq!(payload["order_edit_id"], edit.id.as_str());
    assert_eq!(payload["before_total"], 2_000);
    assert_eq!(payload["after_total"], 3_000);
    assert_eq!(payload["difference_total"], 1_000);

    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE event_type = 'order.edit_confirmed'")
            .fetch_one(fixture.db.pool())
            .await
            .unwrap();
    assert_eq!(count, 1, "a replay must not republish the event");
}
