//! Store-settings domain types.
//!
//! Settings are opaque key/value pairs used for store configuration. Sales
//! channels describe the surfaces (web, marketplace, POS) through which a store
//! sells its catalog.

use fastcart_kernel::Metadata;
use serde::{Deserialize, Serialize};

/// A channel through which a store exposes products.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SalesChannel {
    /// Channel identifier.
    pub id: String,
    /// Human-readable channel name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Disabled channels cannot be used for storefront checkout.
    pub is_disabled: bool,
    /// Free-form channel metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

impl SalesChannel {
    /// Returns true when the channel may be used by storefronts.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !self.is_disabled
    }
}

/// Input for creating a sales channel.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateSalesChannelInput {
    /// Required, unique channel name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
}

/// Input for partially updating a sales channel.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateSalesChannelInput {
    /// New unique name. `None` leaves the name unchanged.
    pub name: Option<String>,
    /// New description. `Some("")` clears it; `None` leaves it unchanged.
    pub description: Option<String>,
    /// Replacement metadata. `None` leaves it unchanged.
    pub metadata: Option<Metadata>,
}
