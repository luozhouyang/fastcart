//! Public API request and response DTOs.
//!
//! Domain entities are never exposed directly: DTOs keep the HTTP contract
//! stable and prevent internal fields (database keys, credential hashes) from
//! leaking.

use fastcart_kernel::{Metadata, PageResult, Pagination};
use fastcart_module_cart::Cart;
use fastcart_module_catalog::{
    Category, Collection, Dimensions, Product, ProductStatus, ProductVariant,
};
use fastcart_module_checkout::{CheckoutSession, CheckoutStateHistory};
use fastcart_module_content::{NavigationTree, Page};
use fastcart_module_customer::{Address, Customer, CustomerGroup, CustomerStatus};
use fastcart_module_order::{ClaimItemType, Order, OrderEditAction};
use fastcart_module_shipping::ResolvedShippingOption;
use fastcart_module_store::Region;
use fastcart_module_theme::Theme;
use fastcart_platform::{Membership, Store, StoreDomain, User};
use fastcart_storefront::ThemeManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::routes::store_catalog::StoreProduct;
use crate::routes::store_inventory::VariantAvailability;

mod catalog;
mod checkout;
mod content;
mod customers;
mod orders;
mod payments;
mod platform;
mod store;

pub use catalog::*;
pub use checkout::*;
pub use content::*;
pub use customers::*;
pub use orders::*;
pub use payments::*;
pub use platform::*;
pub use store::*;
