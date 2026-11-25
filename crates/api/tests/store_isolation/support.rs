use super::*;

pub(crate) struct Fixture {
    pub(crate) app: Router,
    pub(crate) context: Arc<AppContext>,
    pub(crate) admin_auth: String,
    pub(crate) csrf: String,
    pub(crate) store_a: StoreId,
    pub(crate) store_b: StoreId,
    pub(crate) _dir: tempfile::TempDir,
}

pub(crate) async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let config = ApiConfig {
        cookie_secure: false,
        backup_dir: dir.path().join("backups"),
        ..ApiConfig::default()
    };
    let context = Arc::new(AppContext::new(
        config,
        platform,
        databases,
        StoreCache::default_capacity(),
    ));
    let app = build_router(context.clone());

    let (status, _, cookies) = request(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": PASSWORD
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let admin_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let store_a = create_store(
        &app,
        &admin_auth,
        &csrf,
        "Alpha",
        "alpha",
        "alpha.example.com",
    )
    .await;
    let store_b = create_store(&app, &admin_auth, &csrf, "Beta", "beta", "beta.example.com").await;

    Fixture {
        app,
        context,
        admin_auth,
        csrf,
        store_a,
        store_b,
        _dir: dir,
    }
}

pub(crate) async fn create_store(
    app: &Router,
    admin_auth: &str,
    csrf: &str,
    name: &str,
    slug: &str,
    domain: &str,
) -> StoreId {
    let (status, store, _) = request(
        app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": name,
            "slug": slug,
            "primary_domain": domain
        })),
        &[("cookie", admin_auth), ("x-csrf-token", csrf)],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    StoreId::from_string(store["id"].as_str().unwrap())
}

pub(crate) async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value, Vec<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value, cookies)
}

pub(crate) fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

pub(crate) fn admin_headers<'a>(fixture: &'a Fixture, store: &'a str) -> [(&'a str, &'a str); 3] {
    [
        ("cookie", fixture.admin_auth.as_str()),
        ("x-csrf-token", fixture.csrf.as_str()),
        ("x-store-id", store),
    ]
}

pub(crate) async fn seed_product(
    context: &AppContext,
    store_id: &StoreId,
    title: &str,
    sku: &str,
    publish: bool,
) -> Product {
    let db = context.databases.database(store_id).await.unwrap();
    let catalog = CatalogService::new(db);
    let product = catalog
        .create_product(CreateProductInput {
            title: title.to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                sku: Some(sku.to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    if publish {
        catalog.publish_product(&product.id).await.unwrap();
    }
    product
}

pub(crate) async fn seed_order(context: &AppContext, store_id: &StoreId, email: &str) -> Order {
    let db = context.databases.database(store_id).await.unwrap();
    OrderService::new(db)
        .create_order(CreateOrderInput {
            email: email.to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Isolation Hoodie".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                ..Default::default()
            }],
            subtotal: 1_000,
            total: 1_000,
            ..Default::default()
        })
        .await
        .unwrap()
}

pub(crate) async fn seed_storefront_catalog(
    context: &AppContext,
    store_id: &StoreId,
    title: &str,
    sku: &str,
) -> (Product, SalesChannel, CreatedPublishableApiKey) {
    let db = context.databases.database(store_id).await.unwrap();
    let channel = StoreService::new(db.clone())
        .create_sales_channel(CreateSalesChannelInput {
            name: format!("{title} channel"),
            description: None,
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    let key = RegionService::new(db.clone())
        .create_publishable_key(CreatePublishableApiKeyInput {
            title: format!("{title} key"),
            sales_channel_ids: vec![channel.id.clone()],
        })
        .await
        .unwrap();
    let catalog = CatalogService::new(db);
    let product = catalog
        .create_product(CreateProductInput {
            title: title.to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                sku: Some(sku.to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    catalog
        .set_product_sales_channels(&product.id, std::slice::from_ref(&channel.id))
        .await
        .unwrap();
    (product, channel, key)
}
