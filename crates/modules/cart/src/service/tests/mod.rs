use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    CurrencyCode, CustomerId, LineItemId, Metadata, Pagination, ProductId, PromotionId,
    ShippingOptionId, StoreId, VariantId, now_timestamp,
};

use super::CartService;
use crate::domain::{
    AddItemInput, AddressInput, AdjustmentInput, Cart, CartStatus, CreateCartInput, DiscountInput,
    SetShippingMethodInput, TaxLineInput, UpdateItemInput,
};

async fn service() -> CartService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    CartService::new(db)
}

fn create_input(currency: &str) -> CreateCartInput {
    CreateCartInput::new(currency)
}

async fn create_cart(service: &CartService, currency: &str) -> Cart {
    service.create_cart(create_input(currency)).await.unwrap()
}

fn variant() -> VariantId {
    VariantId::new()
}

fn item_input(
    currency: &str,
    variant_id: VariantId,
    unit_price: i64,
    quantity: i64,
) -> AddItemInput {
    AddItemInput {
        product_id: ProductId::new(),
        variant_id,
        title: "Test product".to_owned(),
        variant_title: None,
        sku: Some("SKU-1".to_owned()),
        thumbnail: None,
        unit_price,
        currency_code: currency.to_owned(),
        quantity,
        is_tax_inclusive: false,
        requires_shipping: true,
        metadata: Metadata::empty(),
    }
}

fn shipping_input(currency: &str, amount: i64) -> SetShippingMethodInput {
    SetShippingMethodInput {
        shipping_option_id: ShippingOptionId::new(),
        name: "Standard".to_owned(),
        amount,
        currency_code: currency.to_owned(),
        is_tax_inclusive: false,
        data: serde_json::json!({ "provider": "test" }),
    }
}

fn adjustment(amount: i64) -> AdjustmentInput {
    AdjustmentInput {
        promotion_id: Some(PromotionId::new()),
        code: Some("PROMO".to_owned()),
        amount,
        description: Some("Line promotion".to_owned()),
    }
}

fn tax_line(amount: i64, inclusive: bool) -> TaxLineInput {
    TaxLineInput {
        name: "Sales tax".to_owned(),
        rate_bps: 1_000,
        amount,
        is_tax_inclusive: inclusive,
        cart_item_id: None,
        shipping_method_id: None,
    }
}

mod discounts;
mod gift_cards;
mod items;
mod lifecycle;
mod parties;
mod recalculation;
mod shipping;
mod totals;
