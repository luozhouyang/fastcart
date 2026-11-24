use super::*;

#[tokio::test]
async fn claims_respect_returned_quantities_and_transitions() {
    let service = service().await;
    let order = order(&service, 4).await;
    let returned = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap();
    service
        .request_return(&order.id, &returned.id)
        .await
        .unwrap();

    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 3,
                    item_type: ClaimItemType::Refund,
                    replacement_variant_id: None,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(claim.status, ClaimStatus::Requested);

    let error = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
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
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let accepted = service
        .accept_claim(&order.id, &claim.id, &[])
        .await
        .unwrap();
    assert_eq!(accepted.status, ClaimStatus::Accepted);
    let again = service
        .accept_claim(&order.id, &claim.id, &[])
        .await
        .unwrap();
    assert_eq!(again.status, ClaimStatus::Accepted);

    let error = service
        .cancel_claim(&order.id, &claim.id)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn a_claimed_unit_cannot_also_be_returned() {
    let service = service().await;
    let order = order(&service, 1).await;
    service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
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

    let error = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
