//! Inventory domain entities, inputs and pure availability rules.

use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    ReservationId, ReturnId, Timestamp, VariantId, validate_length, validate_quantity,
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

/// One returned quantity to place back into stock.
///
/// `received_quantity` is the cumulative quantity received on the return line
/// after this transition. It makes the movement reference deterministic, so
/// replaying the same receive cannot increase stock more than once while
/// separate partial receipts still record distinct movements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReturnRestockLine {
    /// Historical order line being returned.
    pub order_item_id: LineItemId,
    /// Catalog variant whose inventory item receives the stock.
    pub variant_id: VariantId,
    /// Quantity received in this transition.
    pub quantity: i64,
    /// Cumulative received quantity for this return line.
    pub received_quantity: i64,
}

impl ReturnRestockLine {
    /// Creates a restock line for a newly received return quantity.
    #[must_use]
    pub fn new(
        order_item_id: LineItemId,
        variant_id: VariantId,
        quantity: i64,
        received_quantity: i64,
    ) -> Self {
        Self {
            order_item_id,
            variant_id,
            quantity,
            received_quantity,
        }
    }
}

/// Builds the deterministic movement reference used by [`ReturnRestockLine`].
#[must_use]
pub fn return_restock_reference(
    return_id: &ReturnId,
    order_item_id: &LineItemId,
    received_quantity: i64,
) -> String {
    format!(
        "return-restock:{}:{}:{}",
        return_id.as_str(),
        order_item_id.as_str(),
        received_quantity
    )
}

/// One inbound exchange quantity to place back into stock.
///
/// `exchange_item_id` identifies the exchange line, which makes the movement
/// reference stable across retries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExchangeRestockLine {
    /// Exchange line whose inbound goods arrived.
    pub exchange_item_id: String,
    /// Catalog variant whose inventory item receives the stock.
    pub variant_id: VariantId,
    /// Quantity received in this transition.
    pub quantity: i64,
}

impl ExchangeRestockLine {
    /// Creates a restock line for an accepted inbound exchange item.
    #[must_use]
    pub fn new(exchange_item_id: impl Into<String>, variant_id: VariantId, quantity: i64) -> Self {
        Self {
            exchange_item_id: exchange_item_id.into(),
            variant_id,
            quantity,
        }
    }
}

/// Builds the deterministic movement reference used by [`ExchangeRestockLine`].
#[must_use]
pub fn exchange_restock_reference(exchange_id: &str, exchange_item_id: &str) -> String {
    format!("exchange-restock:{exchange_id}:{exchange_item_id}")
}

/// One variant quantity consumed from stock by an after-sales replacement.
///
/// The caller supplies a unique `reference` per consumed line; replaying the
/// same reference never removes stock twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockConsumption {
    /// Catalog variant whose inventory item is consumed.
    pub variant_id: VariantId,
    /// Positive quantity to consume.
    pub quantity: i64,
    /// Deterministic idempotency reference for this consumption.
    pub reference: String,
}

impl StockConsumption {
    /// Creates a stock consumption line.
    #[must_use]
    pub fn new(variant_id: VariantId, quantity: i64, reference: impl Into<String>) -> Self {
        Self {
            variant_id,
            quantity,
            reference: reference.into(),
        }
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
