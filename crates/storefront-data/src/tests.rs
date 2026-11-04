//! Focused resolver tests backed by isolated in-memory store databases.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CurrencyCode, StoreId, VariantId};
use fastcart_module_asset::{AssetService, CreateAssetInput};
use fastcart_module_catalog::{
    CatalogService, CreateCollectionInput, CreateProductInput, CreateVariantInput,
};
use fastcart_module_inquiry::{CreateInquiryFormInput, InquiryFormService};
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_storefront::{BindingRequest, DataBinding};

use super::{DataMode, DataResolutionContext, StorefrontDataResolver, resolve_fragment};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").expect("USD is a valid currency")
}

fn context(mode: DataMode) -> DataResolutionContext {
    DataResolutionContext {
        currency: usd(),
        sales_channel_id: None,
        customer_group_id: None,
        mode,
        now: 1_000,
    }
}

fn collection_binding(id: &str) -> BindingRequest {
    BindingRequest {
        block_id: "grid-1".to_owned(),
        field: "collection".to_owned(),
        binding: DataBinding::Collection {
            id: id.to_owned(),
            limit: Some(4),
        },
    }
}

/// Seeds one collection with one published product, a price and stock.
async fn seed_collection(db: &StoreDatabase, published: bool) -> (String, String, String) {
    let catalog = CatalogService::new(db.clone());
    let collection = catalog
        .create_collection(CreateCollectionInput {
            title: "Featured".to_owned(),
            is_published: Some(published),
            ..CreateCollectionInput::default()
        })
        .await
        .expect("collection");
    let product = catalog
        .create_product(CreateProductInput {
            title: "Tee".to_owned(),
            thumbnail: Some("https://cdn.example/tee.png".to_owned()),
            variants: vec![CreateVariantInput {
                title: "Small".to_owned(),
                sku: Some("TEE-S".to_owned()),
                ..CreateVariantInput::default()
            }],
            collection_ids: vec![collection.id.clone()],
            ..CreateProductInput::default()
        })
        .await
        .expect("product");
    catalog
        .publish_product(&product.id)
        .await
        .expect("publish product");

    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(variant_id.clone(), usd(), 1_999))
        .await
        .expect("price");

    let inventory = InventoryService::new(db.clone());
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .expect("inventory item");
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .expect("location");
    inventory
        .adjust_stock(&item.id, &location.id, 5, "test", None)
        .await
        .expect("stock");

    (collection.id, product.id, variant_id)
}

#[tokio::test]
async fn collection_binding_resolves_collection_and_products_with_prices() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let (collection_id, _, variant_id) = seed_collection(&db, true).await;
    let resolver = StorefrontDataResolver::new();

    let bag = resolver
        .resolve(
            &db,
            &[collection_binding(&collection_id)],
            &context(DataMode::Production),
        )
        .await
        .expect("resolution");

    let block = &bag["grid-1"];
    assert_eq!(block["collection"]["id"], collection_id);
    assert_eq!(block["collection"]["title"], "Featured");
    assert_eq!(block["collection"]["handle"], "featured");

    let products = block["products"].as_array().expect("products");
    assert_eq!(products.len(), 1);
    assert_eq!(products[0]["title"], "Tee");
    assert_eq!(products[0]["price"]["amount_minor"], 1_999);
    assert_eq!(products[0]["price"]["amount"], "19.99");
    assert_eq!(products[0]["price"]["currency"], "USD");
    assert_eq!(products[0]["available"], true);
    assert_eq!(products[0]["variants"][0]["id"], variant_id);
    assert_eq!(products[0]["variants"][0]["sku"], "TEE-S");
}

#[tokio::test]
async fn unpublished_collection_degrades_in_production_and_resolves_in_preview() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let (collection_id, _, _) = seed_collection(&db, false).await;
    let resolver = StorefrontDataResolver::new();

    let production = resolver
        .resolve(
            &db,
            &[collection_binding(&collection_id)],
            &context(DataMode::Production),
        )
        .await
        .expect("production resolution");
    assert!(production["grid-1"]["collection"].is_null());
    assert!(
        production["grid-1"]["products"]
            .as_array()
            .expect("products")
            .is_empty()
    );

    let preview = resolver
        .resolve(
            &db,
            &[collection_binding(&collection_id)],
            &context(DataMode::Preview),
        )
        .await
        .expect("preview resolution");
    assert_eq!(preview["grid-1"]["collection"]["id"], collection_id);
    assert_eq!(
        preview["grid-1"]["products"]
            .as_array()
            .expect("products")
            .len(),
        1
    );
}

#[tokio::test]
async fn product_list_inherits_collection_visibility() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let (collection_id, _, _) = seed_collection(&db, false).await;
    let binding = BindingRequest {
        block_id: "grid-2".to_owned(),
        field: "products".to_owned(),
        binding: DataBinding::ProductList {
            collection_id: Some(collection_id),
            tag: None,
            sort: None,
            limit: Some(4),
        },
    };

    let production = StorefrontDataResolver::new()
        .resolve(
            &db,
            std::slice::from_ref(&binding),
            &context(DataMode::Production),
        )
        .await
        .expect("production resolution");
    assert!(
        production["grid-2"]["products"]
            .as_array()
            .expect("products")
            .is_empty(),
        "an unpublished collection must hide its products in production"
    );

    let preview = StorefrontDataResolver::new()
        .resolve(&db, &[binding], &context(DataMode::Preview))
        .await
        .expect("preview resolution");
    assert_eq!(
        preview["grid-2"]["products"]
            .as_array()
            .expect("products")
            .len(),
        1
    );
}

#[tokio::test]
async fn product_list_honours_declared_sort_order() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let catalog = CatalogService::new(db.clone());
    for title in ["Beta", "Alpha", "Gamma"] {
        let product = catalog
            .create_product(CreateProductInput {
                title: title.to_owned(),
                ..CreateProductInput::default()
            })
            .await
            .expect("product");
        catalog.publish_product(&product.id).await.expect("publish");
    }
    let binding = BindingRequest {
        block_id: "grid-3".to_owned(),
        field: "products".to_owned(),
        binding: DataBinding::ProductList {
            collection_id: None,
            tag: None,
            sort: Some("title_desc".to_owned()),
            limit: Some(10),
        },
    };

    let bag = StorefrontDataResolver::new()
        .resolve(&db, &[binding], &context(DataMode::Production))
        .await
        .expect("resolution");
    let titles: Vec<&str> = bag["grid-3"]["products"]
        .as_array()
        .expect("products")
        .iter()
        .map(|product| product["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Gamma", "Beta", "Alpha"]);
}

#[tokio::test]
async fn deleted_or_missing_references_return_null_fragments() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let product = CatalogService::new(db.clone())
        .create_product(CreateProductInput {
            title: "Gone".to_owned(),
            ..CreateProductInput::default()
        })
        .await
        .expect("product");
    CatalogService::new(db.clone())
        .delete_product(&product.id)
        .await
        .expect("delete product");

    let bindings = vec![
        BindingRequest {
            block_id: "deleted-product".to_owned(),
            field: "product".to_owned(),
            binding: DataBinding::Product {
                id: product.id.clone(),
            },
        },
        BindingRequest {
            block_id: "missing-product".to_owned(),
            field: "product".to_owned(),
            binding: DataBinding::Product {
                id: "prod_missing".to_owned(),
            },
        },
        BindingRequest {
            block_id: "missing-page".to_owned(),
            field: "page".to_owned(),
            binding: DataBinding::Page {
                id: "page_missing".to_owned(),
            },
        },
        BindingRequest {
            block_id: "missing-collection".to_owned(),
            field: "collection".to_owned(),
            binding: DataBinding::Collection {
                id: "coll_missing".to_owned(),
                limit: None,
            },
        },
    ];

    let bag = StorefrontDataResolver::new()
        .resolve(&db, &bindings, &context(DataMode::Production))
        .await
        .expect("resolution");

    assert!(bag["deleted-product"]["product"].is_null());
    assert!(bag["missing-product"]["product"].is_null());
    assert!(bag["missing-page"]["page"].is_null());
    assert!(bag["missing-collection"]["collection"].is_null());
    assert!(
        bag["missing-collection"]["products"]
            .as_array()
            .expect("products")
            .is_empty()
    );
}

#[test]
fn binding_concurrency_is_bounded() {
    assert_eq!(super::MAX_CONCURRENT_BINDINGS, 8);
}

#[tokio::test]
async fn one_failed_binding_does_not_drop_other_fragments() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let (collection_id, _, _) = seed_collection(&db, true).await;
    // Removing a table needed by the product listing makes the collection
    // binding fail with a database error rather than a "not found" degradation.
    sqlx::query("DROP TABLE products")
        .execute(db.pool())
        .await
        .expect("drop products");

    let broken = collection_binding(&collection_id);
    let healthy = BindingRequest {
        block_id: "asset-1".to_owned(),
        field: "asset".to_owned(),
        binding: DataBinding::Asset {
            id: "asset-1".to_owned(),
        },
    };

    let bag = StorefrontDataResolver::new()
        .resolve(&db, &[broken, healthy], &context(DataMode::Production))
        .await
        .expect("one failed binding must not fail the bag");

    assert!(!bag.contains_key("grid-1"), "the failed binding is skipped");
    assert_eq!(bag["asset-1"]["asset"], serde_json::Value::Null);
}

#[tokio::test]
async fn all_failed_bindings_surface_the_error() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let (collection_id, _, _) = seed_collection(&db, true).await;
    sqlx::query("DROP TABLE products")
        .execute(db.pool())
        .await
        .expect("drop products");

    let first = collection_binding(&collection_id);
    let mut second = collection_binding(&collection_id);
    second.block_id = "grid-2".to_owned();

    // A page whose every binding fails points at an unavailable data layer,
    // so the resolver must not pretend the page resolved to empty data.
    let result = StorefrontDataResolver::new()
        .resolve(&db, &[first, second], &context(DataMode::Production))
        .await;
    assert!(result.is_err(), "all-failed resolution must not be silent");
}

async fn seed_inquiry_form(db: &StoreDatabase) -> fastcart_module_inquiry::InquiryForm {
    let service = InquiryFormService::new(db.clone());
    let form = service
        .create_form(CreateInquiryFormInput {
            name: "Contact".to_owned(),
            handle: Some("contact".to_owned()),
            description: None,
        })
        .await
        .expect("form");
    service.publish(&form.id, None).await.expect("publish")
}

fn inquiry_form_binding(handle: &str) -> BindingRequest {
    BindingRequest {
        block_id: "form-1".to_owned(),
        field: "form".to_owned(),
        binding: DataBinding::InquiryForm {
            handle: handle.to_owned(),
        },
    }
}

#[tokio::test]
async fn inquiry_form_binding_resolves_fields_settings_and_version() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let form = seed_inquiry_form(&db).await;

    let bag = StorefrontDataResolver::new()
        .resolve(
            &db,
            &[inquiry_form_binding("contact")],
            &context(DataMode::Production),
        )
        .await
        .expect("resolution");

    let resolved = &bag["form-1"]["form"];
    assert_eq!(resolved["handle"], "contact");
    assert_eq!(
        resolved["version"],
        form.published_version.expect("published version"),
        "the resolver reports the published revision version"
    );
    let fields = resolved["fields"].as_array().expect("fields");
    assert!(!fields.is_empty());
    assert!(fields.iter().any(|field| field["key"] == "email"));
    assert!(resolved["settings"]["submit_label"].is_string());
    assert!(resolved["settings"]["success_message"].is_string());
    assert_eq!(resolved["settings"]["allow_attachments"], true);

    let serialized = serde_json::to_string(&bag).expect("serialized bag");
    assert!(
        !serialized.contains("notification_fields"),
        "notification fields must never reach the storefront: {serialized}"
    );
}

#[tokio::test]
async fn missing_inquiry_form_handle_is_not_found_and_skipped() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let binding = inquiry_form_binding("missing");

    let error = resolve_fragment(&db, &binding.binding, &context(DataMode::Production))
        .await
        .expect_err("missing handles must surface a not-found error");
    assert!(error.is_not_found(), "unexpected error: {error}");

    let bag = StorefrontDataResolver::new()
        .resolve(&db, &[binding], &context(DataMode::Production))
        .await
        .expect("a missing reference is skipped rather than failing the page");
    assert!(bag.is_empty());
}

#[tokio::test]
async fn asset_binding_resolves_public_metadata() {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("database");
    let asset = AssetService::new(db.clone())
        .create_asset(CreateAssetInput {
            filename: "hero.png".to_owned(),
            content_type: "image/png".to_owned(),
            size_bytes: 128,
            storage_key: "assets/store/hero.png".to_owned(),
            alt: Some("Hero image".to_owned()),
            width: Some(1200),
            height: Some(600),
            created_by: None,
        })
        .await
        .expect("asset");
    let binding = BindingRequest {
        block_id: "hero".to_owned(),
        field: "image".to_owned(),
        binding: DataBinding::Asset {
            id: asset.id.clone(),
        },
    };

    let bag = StorefrontDataResolver::new()
        .resolve(&db, &[binding], &context(DataMode::Production))
        .await
        .expect("resolution");

    assert_eq!(bag["hero"]["asset"]["id"], asset.id);
    assert_eq!(
        bag["hero"]["asset"]["url"],
        format!("/api/v1/store/assets/{}", asset.id)
    );
    assert_eq!(bag["hero"]["asset"]["alt"], "Hero image");
    assert_eq!(bag["hero"]["asset"]["width"], 1200);
    assert_eq!(bag["hero"]["asset"]["height"], 600);
}

#[tokio::test]
async fn cache_does_not_leak_between_stores() {
    let store_a = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("store a");
    let store_b = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("store b");
    let (collection_id, _, _) = seed_collection(&store_a, true).await;

    let resolver = StorefrontDataResolver::new();
    let binding = collection_binding(&collection_id);
    let request = std::slice::from_ref(&binding);

    let from_a = resolver
        .resolve(&store_a, request, &context(DataMode::Production))
        .await
        .expect("store a resolution");
    assert_eq!(from_a["grid-1"]["collection"]["id"], collection_id);

    let from_b = resolver
        .resolve(&store_b, request, &context(DataMode::Production))
        .await
        .expect("store b resolution");
    assert!(from_b["grid-1"]["collection"].is_null());
    assert!(
        from_b["grid-1"]["products"]
            .as_array()
            .expect("products")
            .is_empty()
    );
}
