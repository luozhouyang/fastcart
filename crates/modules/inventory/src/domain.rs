//! Inventory domain entities, inputs and pure availability rules.

use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    ReservationId, Timestamp, VariantId, validate_length, validate_quantity,
};

/// A warehouse, store or other physical location that holds stock.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InventoryLocation {
    /// Location identifier.
    pub id: InventoryLocationId,
    /// Human-readable name.
    pub name: String,
    /// Structured postal address.
    pub address: Metadata,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating an inventory location.
#[derive(Clone, Debug, Default)]
pub struct CreateLocationInput {
    /// Human-readable name.
    pub name: String,
    /// Structured postal address.
    pub address: Option<Metadata>,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Input for partially updating an inventory location.
#[derive(Clone, Debug, Default)]
pub struct UpdateLocationInput {
    /// Replacement name.
    pub name: Option<String>,
    /// Replacement address.
    pub address: Option<Metadata>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// An inventory item, optionally linked to a catalog variant and/or SKU.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InventoryItem {
    /// Item identifier.
    pub id: InventoryItemId,
    /// Stock keeping unit, unique when present.
    pub sku: Option<String>,
    /// Catalog variant backed by this inventory item.
    pub variant_id: Option<VariantId>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating an inventory item.
#[derive(Clone, Debug, Default)]
pub struct CreateInventoryItemInput {
    /// Stock keeping unit.
    pub sku: Option<String>,
    /// Catalog variant identifier.
    pub variant_id: Option<VariantId>,
    /// Extension metadata.
    pub metadata: Option<Metadata>,
}

/// Stock held for one item at one location.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InventoryLevel {
    /// Level identifier.
    pub id: String,
    /// Inventory item.
    pub inventory_item_id: InventoryItemId,
    /// Location holding the stock.
    pub location_id: InventoryLocationId,
    /// On-hand quantity.
    pub stocked_quantity: i64,
    /// Quantity reserved by pending checkouts.
    pub reserved_quantity: i64,
    /// Quantity expected to arrive.
    pub incoming_quantity: i64,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

impl InventoryLevel {
    /// Quantity that can still be promised to a new checkout.
    #[must_use]
    pub fn available_quantity(&self) -> i64 {
        available_quantity_from(self.stocked_quantity, self.reserved_quantity)
    }
}

/// Central availability rule: on-hand minus reserved, never below zero.
#[must_use]
pub fn available_quantity_from(stocked_quantity: i64, reserved_quantity: i64) -> i64 {
    stocked_quantity.saturating_sub(reserved_quantity).max(0)
}

/// Lifecycle state of an inventory reservation.
#[derive(
    utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ReservationStatus {
    /// Stock is held but not yet shipped.
    Reserved,
    /// Stock hold was cancelled or expired.
    Released,
    /// Stock left the warehouse and was recorded as sold.
    Committed,
}

impl ReservationStatus {
    /// Storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Released => "released",
            Self::Committed => "committed",
        }
    }

    /// Parses a stored status value.
    #[must_use]
    pub fn from_db(value: &str) -> Self {
        match value {
            "released" => Self::Released,
            "committed" => Self::Committed,
            _ => Self::Reserved,
        }
    }
}

/// A quantity of one inventory item held at one location.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InventoryReservation {
    /// Reservation identifier.
    pub id: ReservationId,
    /// Inventory item reserved.
    pub inventory_item_id: InventoryItemId,
    /// Location the stock is held at.
    pub location_id: InventoryLocationId,
    /// Reserved quantity.
    pub quantity: i64,
    /// Reservation lifecycle state.
    pub status: ReservationStatus,
    /// Cart or order line that caused the reservation.
    pub line_item_id: Option<LineItemId>,
    /// Checkout owning the reservation.
    pub checkout_id: Option<CheckoutId>,
    /// Order that committed the reservation.
    pub order_id: Option<OrderId>,
    /// Expiry timestamp in Unix seconds.
    pub expires_at: Option<Timestamp>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// One requested variant quantity in a reservation request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservationLine {
    /// Variant to reserve.
    pub variant_id: VariantId,
    /// Positive quantity requested.
    pub quantity: i64,
    /// Optional cart/order line owning the reservation.
    pub line_item_id: Option<LineItemId>,
}

impl ReservationLine {
    /// Creates a reservation line.
    #[must_use]
    pub fn new(variant_id: VariantId, quantity: i64) -> Self {
        Self {
            variant_id,
            quantity,
            line_item_id: None,
        }
    }
}

/// The checkout that owns a group of reservations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservationReference {
    /// Checkout identifier.
    pub checkout_id: CheckoutId,
}

impl ReservationReference {
    /// Creates a reservation reference for a checkout.
    #[must_use]
    pub fn new(checkout_id: CheckoutId) -> Self {
        Self { checkout_id }
    }
}

/// Alias used by callers that model the owning checkout as a reference.
pub type Reference = ReservationReference;

/// An audit record for a change in stocked quantity.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InventoryMovement {
    /// Movement identifier.
    pub id: String,
    /// Inventory item affected.
    pub inventory_item_id: InventoryItemId,
    /// Location affected.
    pub location_id: InventoryLocationId,
    /// Signed change in stocked quantity.
    pub quantity_delta: i64,
    /// Short machine-readable reason.
    pub reason: String,
    /// Optional external reference such as an order identifier.
    pub reference: Option<String>,
    /// Optional actor identifier.
    pub actor_id: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// Validates a location name.
pub fn validate_name(name: &str, field: &str) -> Result<(), AppError> {
    validate_length(name, field, 200)
}

/// Validates an adjustment or transfer movement reason.
pub fn validate_reason(reason: &str) -> Result<(), AppError> {
    validate_length(reason, "reason", 120)
}

/// Validates a positive reservation or transfer quantity.
pub fn validate_positive_quantity(quantity: i64, field: &str) -> Result<(), AppError> {
    validate_quantity(quantity, false)
        .map_err(|_| AppError::validation(format!("{field} must be between 1 and 1000000")))
}

/// Validates a reservation time-to-live in seconds.
pub fn validate_ttl(ttl_seconds: i64) -> Result<(), AppError> {
    if ttl_seconds <= 0 {
        return Err(AppError::validation("ttl_seconds must be positive"));
    }
    Ok(())
}
