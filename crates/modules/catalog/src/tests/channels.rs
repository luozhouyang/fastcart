use super::*;

#[tokio::test]
async fn sets_and_replaces_product_sales_channels() {
    let (service, db) = service_with_db().await;
    insert_sales_channel(&db, "sc_pos").await;
    insert_sales_channel(&db, "sc_web").await;
    let product = basic_product(&service, "Channeled", "CH-1").await;
    assert!(product.sales_channel_ids.is_empty());

    service
        .set_product_sales_channels(
            &product.id,
            &[
                "sc_web".to_owned(),
                "sc_pos".to_owned(),
                "sc_web".to_owned(),
            ],
        )
        .await
        .unwrap();
    let loaded = service.get_product(&product.id).await.unwrap();
    assert_eq!(
        loaded.sales_channel_ids,
        vec!["sc_pos".to_owned(), "sc_web".to_owned()]
    );
    assert_eq!(
        service
            .list_product_sales_channel_ids(&product.id)
            .await
            .unwrap(),
        vec!["sc_pos".to_owned(), "sc_web".to_owned()]
    );

    service
        .set_product_sales_channels(&product.id, &["sc_pos".to_owned()])
        .await
        .unwrap();
    assert_eq!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .sales_channel_ids,
        vec!["sc_pos".to_owned()]
    );

    service
        .set_product_sales_channels(&product.id, &[])
        .await
        .unwrap();
    assert!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .sales_channel_ids
            .is_empty()
    );
}

#[tokio::test]
async fn rejects_unknown_sales_channels() {
    let (service, db) = service_with_db().await;
    insert_sales_channel(&db, "sc_web").await;
    let product = basic_product(&service, "Channeled", "CH-2").await;

    assert!(matches!(
        service
            .set_product_sales_channels(
                &product.id,
                &["sc_web".to_owned(), "sc_missing".to_owned()]
            )
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .sales_channel_ids
            .is_empty()
    );
    assert!(
        service
            .set_product_sales_channels("prod_missing", &["sc_web".to_owned()])
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn filters_and_searches_products_by_sales_channel() {
    let (service, db) = service_with_db().await;
    insert_sales_channel(&db, "sc_pos").await;
    insert_sales_channel(&db, "sc_web").await;

    let web = service
        .create_product(CreateProductInput {
            title: "Red Shirt".to_owned(),
            variants: vec![variant_input("Default", "SC-1")],
            ..Default::default()
        })
        .await
        .unwrap();
    let pos = service
        .create_product(CreateProductInput {
            title: "Blue Shirt".to_owned(),
            variants: vec![variant_input("Default", "SC-2")],
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .set_product_sales_channels(&web.id, &["sc_web".to_owned()])
        .await
        .unwrap();
    service
        .set_product_sales_channels(&pos.id, &["sc_pos".to_owned()])
        .await
        .unwrap();
    service.publish_product(&web.id).await.unwrap();
    service.publish_product(&pos.id).await.unwrap();

    let web_only = service
        .list_products(
            ProductFilter {
                sales_channel_ids: Some(vec!["sc_web".to_owned()]),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(web_only.count, 1);
    assert_eq!(web_only.data[0].id, web.id);
    assert_eq!(
        web_only.data[0].sales_channel_ids,
        vec!["sc_web".to_owned()]
    );

    let no_filter = service
        .list_products(
            ProductFilter {
                sales_channel_ids: Some(Vec::new()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(no_filter.count, 2);

    let storefront = service
        .search_products_with_filter(
            "shirt",
            ProductFilter {
                status: Some(ProductStatus::Active),
                sales_channel_ids: Some(vec!["sc_pos".to_owned()]),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(storefront.count, 1);
    assert_eq!(storefront.data[0].id, pos.id);

    assert!(
        service
            .get_published_by_handle_in_channels(&web.handle, &["sc_pos".to_owned()])
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert_eq!(
        service
            .get_published_by_handle_in_channels(&web.handle, &["sc_web".to_owned()])
            .await
            .unwrap()
            .id,
        web.id
    );
    assert_eq!(
        service
            .get_published_by_handle_in_channels(&web.handle, &[])
            .await
            .unwrap()
            .id,
        web.id
    );
}

#[tokio::test]
async fn storefront_sorting_orders_products() {
    let service = service().await;
    basic_product(&service, "Beta", "SORT-B").await;
    basic_product(&service, "Alpha", "SORT-A").await;
    basic_product(&service, "Gamma", "SORT-G").await;

    let ascending = service
        .list_products_sorted(
            ProductFilter::default(),
            Some(ProductSort::TitleAsc),
            Pagination::default(),
        )
        .await
        .unwrap();
    let titles: Vec<&str> = ascending
        .data
        .iter()
        .map(|product| product.title.as_str())
        .collect();
    assert_eq!(titles, ["Alpha", "Beta", "Gamma"]);

    let descending = service
        .list_products_sorted(
            ProductFilter::default(),
            Some(ProductSort::TitleDesc),
            Pagination::default(),
        )
        .await
        .unwrap();
    let titles: Vec<&str> = descending
        .data
        .iter()
        .map(|product| product.title.as_str())
        .collect();
    assert_eq!(titles, ["Gamma", "Beta", "Alpha"]);
}
