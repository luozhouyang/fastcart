use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CurrencyCode, Metadata, Pagination, StoreId};

use super::PricingService;
use crate::domain::{
    CreatePriceInput, CreatePriceListInput, Price, PriceContext, PriceFilter, PriceListRuleInput,
    PriceListStatus, PriceListType, UpdatePriceInput, UpdatePriceListInput, select_price,
};

async fn service() -> PricingService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    PricingService::new(db)
}

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn eur() -> CurrencyCode {
    CurrencyCode::new("EUR").unwrap()
}

fn context(quantity: i64, at: i64) -> PriceContext {
    PriceContext {
        currency: usd(),
        quantity,
        customer_group_id: None,
        sales_channel_id: None,
        at,
    }
}

fn sample_price(id: &str, amount: i64) -> Price {
    Price {
        id: id.to_owned(),
        price_list_id: None,
        price_list_type: None,
        variant_id: "var_1".to_owned(),
        currency_code: usd(),
        amount,
        min_quantity: 1,
        max_quantity: None,
        customer_group_id: None,
        sales_channel_id: None,
        starts_at: None,
        ends_at: None,
        metadata: Metadata::empty(),
        created_at: 0,
        updated_at: 0,
        price_list_rule_count: 0,
    }
}

fn sale_price(id: &str, list_id: &str, amount: i64) -> Price {
    let mut price = sample_price(id, amount);
    price.price_list_id = Some(list_id.to_owned());
    price.price_list_type = Some(PriceListType::Sale);
    price
}

fn override_price(id: &str, list_id: &str, amount: i64) -> Price {
    let mut price = sample_price(id, amount);
    price.price_list_id = Some(list_id.to_owned());
    price.price_list_type = Some(PriceListType::Override);
    price
}

mod calculation;
mod price_lists;
mod prices;
mod selector;
