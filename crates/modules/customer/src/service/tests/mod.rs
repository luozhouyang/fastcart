use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CountryCode, CustomerId, Metadata, Pagination, now_timestamp};

use crate::domain::{
    AddressInput, CreateCustomerInput, CustomerAuthTokenType, CustomerStatus, MAX_NAME_LENGTH,
    MAX_PHONE_LENGTH, RegisterCustomerInput, UpdateCustomerInput,
};

use fastcart_kernel::StoreId;

use super::*;

async fn service() -> (CustomerService, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    (CustomerService::new(db.clone()), db)
}

fn address(address_1: &str) -> AddressInput {
    AddressInput::new(address_1, "Springfield", "12345", "us")
}

mod addresses;
mod auth;
mod customers;
mod groups;
mod reset;
mod tokens;
mod verification;
