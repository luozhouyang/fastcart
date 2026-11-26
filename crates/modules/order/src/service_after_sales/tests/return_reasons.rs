use super::*;

use crate::after_sales::{
    CreateReturnReasonInput, ReturnReasonId, UpdateReturnReasonInput, WriteOffReturnInput,
};

fn reason_input(value: &str, label: &str) -> CreateReturnReasonInput {
    CreateReturnReasonInput {
        value: value.to_owned(),
        label: label.to_owned(),
        description: None,
        metadata: fastcart_kernel::Metadata::default(),
    }
}

#[tokio::test]
async fn return_reasons_support_crud() {
    let service = service().await;

    let created = service
        .create_return_reason(reason_input("damaged", "Arrived damaged"))
        .await
        .unwrap();
    assert_eq!(created.value, "damaged");
    assert_eq!(created.label, "Arrived damaged");
    assert_eq!(created.description, None);

    let fetched = service.get_return_reason(&created.id).await.unwrap();
    assert_eq!(fetched, created);

    let listed = service.list_return_reasons().await.unwrap();
    assert_eq!(listed, vec![created.clone()]);

    let updated = service
        .update_return_reason(
            &created.id,
            UpdateReturnReasonInput {
                value: None,
                label: Some("Damaged in transit".to_owned()),
                description: Some(Some("Carrier damage".to_owned())),
                metadata: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.value, "damaged");
    assert_eq!(updated.label, "Damaged in transit");
    assert_eq!(updated.description.as_deref(), Some("Carrier damage"));
    assert_eq!(updated.created_at, created.created_at);

    // An explicit null description clears it while omitted fields stay.
    let cleared = service
        .update_return_reason(
            &created.id,
            UpdateReturnReasonInput {
                value: None,
                label: None,
                description: Some(None),
                metadata: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.description, None);
    assert_eq!(cleared.label, "Damaged in transit");

    service.delete_return_reason(&created.id).await.unwrap();
    assert!(service.list_return_reasons().await.unwrap().is_empty());
    let error = service.get_return_reason(&created.id).await.unwrap_err();
    assert!(error.is_not_found());
    let error = service.delete_return_reason(&created.id).await.unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn return_reason_values_are_unique() {
    let service = service().await;
    service
        .create_return_reason(reason_input("damaged", "Arrived damaged"))
        .await
        .unwrap();

    let error = service
        .create_return_reason(reason_input("damaged", "Another label"))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    // Trimming happens before the uniqueness check.
    let error = service
        .create_return_reason(reason_input("  damaged  ", "Another label"))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let other = service
        .create_return_reason(reason_input("wrong-size", "Wrong size"))
        .await
        .unwrap();
    let error = service
        .update_return_reason(
            &other.id,
            UpdateReturnReasonInput {
                value: Some("damaged".to_owned()),
                label: None,
                description: None,
                metadata: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let error = service
        .create_return_reason(reason_input("  ", "Blank value"))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
    let error = service
        .create_return_reason(reason_input("no-label", "  "))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}

#[tokio::test]
async fn a_return_snapshots_the_reason_label_and_survives_its_deletion() {
    let service = service().await;
    let order = order(&service, 2).await;
    let reason = service
        .create_return_reason(reason_input("damaged", "Arrived damaged"))
        .await
        .unwrap();

    let created = service
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: Some("free text that is ignored".to_owned()),
                reason_id: Some(reason.id.clone()),
                refund_id: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    // The configured reason's label is the snapshot, not the supplied text.
    assert_eq!(created.reason.as_deref(), Some("Arrived damaged"));

    // Renaming the reason does not rewrite the record.
    service
        .update_return_reason(
            &reason.id,
            UpdateReturnReasonInput {
                value: None,
                label: Some("Renamed".to_owned()),
                description: None,
                metadata: None,
            },
        )
        .await
        .unwrap();
    let reloaded = service.get_return(&order.id, &created.id).await.unwrap();
    assert_eq!(reloaded.reason.as_deref(), Some("Arrived damaged"));

    // Deleting the reason keeps the historical return readable and restock
    // flows operating: only the lookup of the reason itself disappears.
    service.delete_return_reason(&reason.id).await.unwrap();
    let reloaded = service.get_return(&order.id, &created.id).await.unwrap();
    assert_eq!(reloaded.reason.as_deref(), Some("Arrived damaged"));
    service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    let received = service
        .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(received.reason.as_deref(), Some("Arrived damaged"));
    let written_off = service
        .write_off_return(&order.id, &created.id, WriteOffReturnInput::default())
        .await
        .unwrap();
    assert_eq!(written_off.reason.as_deref(), Some("Arrived damaged"));

    // A new return can no longer reference the deleted reason.
    let error = service
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: None,
                reason_id: Some(ReturnReasonId::from_string("rreason_missing")),
                refund_id: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn claims_and_exchanges_snapshot_the_reason_label() {
    let service = service().await;
    let order = order(&service, 4).await;
    let reason = service
        .create_return_reason(reason_input("damaged", "Arrived damaged"))
        .await
        .unwrap();

    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                reason_id: Some(reason.id.clone()),
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    item_type: ClaimItemType::Refund,
                    replacement_variant_id: None,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(claim.reason.as_deref(), Some("Arrived damaged"));

    let exchange = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: Some(reason.id.clone()),
                metadata: fastcart_kernel::Metadata::default(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(exchange.reason.as_deref(), Some("Arrived damaged"));

    service.delete_return_reason(&reason.id).await.unwrap();
    let reloaded = service.get_claim(&order.id, &claim.id).await.unwrap();
    assert_eq!(reloaded.reason.as_deref(), Some("Arrived damaged"));
    let reloaded = service.get_exchange(&order.id, &exchange.id).await.unwrap();
    assert_eq!(reloaded.reason.as_deref(), Some("Arrived damaged"));

    // The free-text reason remains supported when no configured reason is used.
    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: Some("free text".to_owned()),
                reason_id: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    item_type: ClaimItemType::Refund,
                    replacement_variant_id: None,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(claim.reason.as_deref(), Some("free text"));
}
