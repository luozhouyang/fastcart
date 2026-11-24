use super::*;

#[tokio::test]
async fn groups_are_created_assigned_and_removed() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(CreateCustomerInput::new("owner@example.com"))
        .await
        .unwrap();
    let group = service
        .create_group("VIP", Metadata::empty())
        .await
        .unwrap();
    assert_eq!(group.name, "VIP");

    let duplicate = service
        .create_group("VIP", Metadata::empty())
        .await
        .unwrap_err();
    assert!(matches!(duplicate, AppError::Conflict(_)));

    service.assign_group(&customer.id, &group.id).await.unwrap();
    service.assign_group(&customer.id, &group.id).await.unwrap();
    let assigned = service.list_customer_groups(&customer.id).await.unwrap();
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].id, group.id);

    assert_eq!(service.get_group(&group.id).await.unwrap(), group);
    assert_eq!(service.list_groups().await.unwrap().len(), 1);

    service.remove_group(&customer.id, &group.id).await.unwrap();
    assert!(
        service
            .list_customer_groups(&customer.id)
            .await
            .unwrap()
            .is_empty()
    );
}
