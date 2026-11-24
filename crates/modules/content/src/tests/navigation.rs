use super::*;

#[tokio::test]
async fn navigation_crud_and_handle_uniqueness() {
    let service = service().await;
    let navigation = service
        .create_navigation(navigation_input("Main Menu"))
        .await
        .unwrap();
    assert_eq!(navigation.handle, "main-menu");
    assert!(navigation.items.is_empty());

    let fetched = service.get_navigation(&navigation.id).await.unwrap();
    assert_eq!(fetched, navigation);
    let by_handle = service.get_navigation_by_handle("main-menu").await.unwrap();
    assert_eq!(by_handle.id, navigation.id);

    let updated = service
        .update_navigation(
            &navigation.id,
            crate::UpdateNavigationInput {
                name: Some("Footer".to_owned()),
                handle: Some("footer".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Footer");
    assert_eq!(updated.handle, "footer");

    let error = service
        .create_navigation(CreateNavigationInput {
            handle: Some("footer".to_owned()),
            ..navigation_input("Duplicate")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let listed = service.list_navigations(pagination(10, 0)).await.unwrap();
    assert_eq!(listed.count, 1);
    assert_eq!(listed.data[0].id, navigation.id);

    service.delete_navigation(&navigation.id).await.unwrap();
    assert!(
        service
            .get_navigation(&navigation.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn navigation_items_nest_reorder_and_cascade_delete() {
    let service = service().await;
    let navigation = service
        .create_navigation(navigation_input("Main Menu"))
        .await
        .unwrap();

    let products = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                rank: Some(0),
                ..item_input("Products", "/products")
            },
        )
        .await
        .unwrap();
    let shoes = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                parent_id: Some(products.id.clone()),
                rank: Some(5),
                ..item_input("Shoes", "/products/shoes")
            },
        )
        .await
        .unwrap();
    let about = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                rank: Some(1),
                ..item_input("About", "/about")
            },
        )
        .await
        .unwrap();

    let tree = service.get_navigation_tree("main-menu").await.unwrap();
    assert_eq!(tree.items.len(), 2);
    assert_eq!(tree.items[0].id, products.id);
    assert_eq!(tree.items[0].children.len(), 1);
    assert_eq!(tree.items[0].children[0].id, shoes.id);
    assert_eq!(tree.items[1].id, about.id);

    let reordered = service
        .reorder_navigation_items(&navigation.id, &[about.id.clone(), products.id.clone()])
        .await
        .unwrap();
    assert_eq!(reordered[0].id, about.id);
    assert_eq!(reordered[0].rank, 0);
    assert_eq!(reordered[1].id, products.id);
    assert_eq!(reordered[1].rank, 1);

    let updated = service
        .update_navigation_item(
            &about.id,
            NavigationItemInput {
                parent_id: None,
                title: "Company".to_owned(),
                url: "/company".to_owned(),
                rank: Some(2),
                metadata: Metadata::empty(),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Company");
    assert_eq!(updated.url, "/company");
    assert_eq!(updated.rank, 2);

    service.remove_navigation_item(&products.id).await.unwrap();
    let after = service.get_navigation(&navigation.id).await.unwrap();
    assert_eq!(after.items.len(), 1);
    assert_eq!(after.items[0].id, about.id);
    assert!(
        service
            .remove_navigation_item(&shoes.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn rejects_navigation_item_cycles_and_foreign_parents() {
    let service = service().await;
    let first = service
        .create_navigation(navigation_input("First"))
        .await
        .unwrap();
    let second = service
        .create_navigation(navigation_input("Second"))
        .await
        .unwrap();

    let root = service
        .add_navigation_item(&first.id, item_input("Root", "/root"))
        .await
        .unwrap();
    let child = service
        .add_navigation_item(
            &first.id,
            NavigationItemInput {
                parent_id: Some(root.id.clone()),
                ..item_input("Child", "/child")
            },
        )
        .await
        .unwrap();
    let foreign = service
        .add_navigation_item(&second.id, item_input("Foreign", "/foreign"))
        .await
        .unwrap();

    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some(child.id.clone()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some(root.id.clone()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .add_navigation_item(
            &first.id,
            NavigationItemInput {
                parent_id: Some(foreign.id.clone()),
                ..item_input("Cross Nav", "/cross")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some("nitem_missing".to_owned()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn navigation_cycle_check_sees_the_transactional_parent_chain() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let navigation = service
        .create_navigation(navigation_input("Main Menu"))
        .await
        .unwrap();
    let root = service
        .add_navigation_item(&navigation.id, item_input("Root", "/root"))
        .await
        .unwrap();
    let child = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                parent_id: Some(root.id.clone()),
                ..item_input("Child", "/child")
            },
        )
        .await
        .unwrap();
    let grandchild = service
        .add_navigation_item(
            &navigation.id,
            NavigationItemInput {
                parent_id: Some(child.id.clone()),
                ..item_input("Grandchild", "/grandchild")
            },
        )
        .await
        .unwrap();

    // Reparenting the root below its own grandchild must be rejected even
    // though the ancestor chain is several levels deep.
    let error = service
        .update_navigation_item(
            &root.id,
            NavigationItemInput {
                parent_id: Some(grandchild.id.clone()),
                ..item_input("Root", "/root")
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    // The rejected update left the item untouched and wrote no audit entry.
    let navigation = service.get_navigation(&navigation.id).await.unwrap();
    let stored_root = navigation
        .items
        .iter()
        .find(|item| item.id == root.id)
        .expect("root survives a rejected cycle");
    assert!(stored_root.parent_id.is_none());

    let audits = fetch_pending_audits(db.pool(), 20).await.unwrap();
    assert!(
        !audits
            .iter()
            .any(|audit| audit.action == "navigation_item.updated")
    );
}

#[tokio::test]
async fn updating_a_missing_navigation_item_is_not_found_without_audit() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

    let error = service
        .update_navigation_item("nitem_missing", item_input("Missing", "/missing"))
        .await
        .unwrap_err();
    assert!(error.is_not_found());

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert!(audits.is_empty());
}

#[tokio::test]
async fn navigation_handle_races_map_to_conflicts_without_audit() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = ContentService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let first = service
        .create_navigation(navigation_input("First"))
        .await
        .unwrap();
    let second = service
        .create_navigation(navigation_input("Second"))
        .await
        .unwrap();

    let error = service
        .update_navigation(
            &second.id,
            UpdateNavigationInput {
                handle: Some(first.handle.clone()),
                ..UpdateNavigationInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert!(
        !audits
            .iter()
            .any(|audit| audit.action == "navigation.updated")
    );
}

#[tokio::test]
async fn reorder_rejects_duplicate_foreign_and_missing_items() {
    let service = service().await;
    let first = service
        .create_navigation(navigation_input("First"))
        .await
        .unwrap();
    let second = service
        .create_navigation(navigation_input("Second"))
        .await
        .unwrap();
    let first_item = service
        .add_navigation_item(&first.id, item_input("First Item", "/first"))
        .await
        .unwrap();
    let second_item = service
        .add_navigation_item(&second.id, item_input("Second Item", "/second"))
        .await
        .unwrap();

    let error = service
        .reorder_navigation_items(&first.id, &[first_item.id.clone(), first_item.id.clone()])
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .reorder_navigation_items(&first.id, std::slice::from_ref(&second_item.id))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .reorder_navigation_items(&first.id, &["nitem_missing".to_owned()])
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}
