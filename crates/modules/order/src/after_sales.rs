//! Post-purchase domain entities: returns, claims, exchanges and order edits.
//!
//! These flows never rewrite the immutable `order_items` snapshot. Requested,
//! received and written-off quantities, replacement variants and edit deltas
//! are persisted in dedicated tables and exposed through the order aggregate.

use fastcart_kernel::{
    ClaimId, EventId, ExchangeId, LineItemId, Metadata, OrderEditId, OrderId, ProductId, RefundId,
    ReturnId, Timestamp, VariantId,
};
use serde::{Deserialize, Serialize};

/// Generates a string-backed status enum with storage helpers.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $value:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )+
        }

        impl $name {
            /// Stable storage and API representation.
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self {
                    $(
                        Self::$variant => $value,
                    )+
                }
            }

            /// Parses a stored value, rejecting unknown variants.
            pub fn from_stored(value: &str) -> Result<Self, fastcart_kernel::AppError> {
                match value {
                    $(
                        $value => Ok(Self::$variant),
                    )+
                    other => Err(fastcart_kernel::AppError::database(format!(
                        "invalid {} stored in database: {other}",
                        stringify!($name)
                    ))),
                }
            }
        }
    };
}

string_enum! {
    /// Lifecycle state of a return request.
    pub enum ReturnStatus {
        /// Created but not yet submitted to the customer.
        Draft => "draft",
        /// Submitted return request.
        Requested => "requested",
        /// Some but not all requested quantities arrived.
        PartiallyReceived => "partially_received",
        /// Every requested quantity arrived.
        Received => "received",
        /// Received items were written off instead of restocked.
        WrittenOff => "written_off",
        /// Return request was canceled.
        Canceled => "canceled",
    }
}

impl ReturnStatus {
    /// Returns true when the return may still be canceled.
    #[must_use]
    pub fn is_cancelable(self) -> bool {
        matches!(
            self,
            Self::Draft | Self::Requested | Self::PartiallyReceived
        )
    }
}

string_enum! {
    /// Per-line state of a return item.
    pub enum ReturnItemStatus {
        /// Awaiting receipt.
        Requested => "requested",
        /// Fully received.
        Received => "received",
        /// Fully written off.
        WrittenOff => "written_off",
        /// Line was canceled with the return.
        Canceled => "canceled",
    }
}

string_enum! {
    /// Lifecycle state of a claim.
    pub enum ClaimStatus {
        /// Claim requested by the merchant.
        Requested => "requested",
        /// Claim accepted and resolved.
        Accepted => "accepted",
        /// Claim canceled.
        Canceled => "canceled",
    }
}

string_enum! {
    /// Per-line state of a claim item.
    pub enum ClaimItemStatus {
        /// Claim line requested.
        Requested => "requested",
        /// Claim line accepted.
        Accepted => "accepted",
        /// Claim line canceled.
        Canceled => "canceled",
    }
}

string_enum! {
    /// How a claim resolves the affected line.
    pub enum ClaimItemType {
        /// Refund the customer.
        Refund => "refund",
        /// Replace the item with another variant.
        Replace => "replace",
    }
}

string_enum! {
    /// Lifecycle state of an exchange.
    pub enum ExchangeStatus {
        /// Exchange requested.
        Requested => "requested",
        /// Exchange accepted and resolved.
        Accepted => "accepted",
        /// Exchange canceled.
        Canceled => "canceled",
    }
}

string_enum! {
    /// Direction of an exchange line.
    pub enum ExchangeItemDirection {
        /// Item returned by the customer.
        Inbound => "inbound",
        /// Replacement item shipped to the customer.
        Outbound => "outbound",
    }
}

string_enum! {
    /// Per-line state of an exchange item.
    pub enum ExchangeItemStatus {
        /// Exchange line requested.
        Requested => "requested",
        /// Exchange line accepted.
        Accepted => "accepted",
        /// Exchange line canceled.
        Canceled => "canceled",
    }
}

string_enum! {
    /// Lifecycle state of an order edit.
    pub enum OrderEditStatus {
        /// Edit prepared but not applied.
        Draft => "draft",
        /// Edit applied and order totals recalculated.
        Confirmed => "confirmed",
        /// Edit canceled without changing the order.
        Canceled => "canceled",
    }
}

string_enum! {
    /// Kind of change proposed by an order edit line.
    pub enum OrderEditAction {
        /// Add a line that is not part of the historical snapshot.
        Add => "add",
        /// Remove quantity from an existing line.
        Remove => "remove",
        /// Change the quantity of an existing line.
        Update => "update",
    }
}

string_enum! {
    /// Per-line state of an order edit item.
    pub enum OrderEditItemStatus {
        /// Edit line prepared.
        Draft => "draft",
        /// Edit line applied.
        Confirmed => "confirmed",
        /// Edit line canceled.
        Canceled => "canceled",
    }
}

/// A return request for one order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderReturn {
    /// Return identifier.
    pub id: ReturnId,
    /// Owning order.
    pub order_id: OrderId,
    /// Return lifecycle state.
    pub status: ReturnStatus,
    /// Optional reason for the return.
    pub reason: Option<String>,
    /// Refund created for this return, when linked.
    pub refund_id: Option<RefundId>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Submission timestamp in Unix seconds.
    pub requested_at: Option<Timestamp>,
    /// Timestamp at which every requested item was received.
    pub received_at: Option<Timestamp>,
    /// Timestamp at which the return was written off.
    pub written_off_at: Option<Timestamp>,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Returned lines.
    pub items: Vec<OrderReturnItem>,
}

/// One line of a return request.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderReturnItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning return.
    pub return_id: ReturnId,
    /// Historical order line being returned.
    pub order_item_id: LineItemId,
    /// Requested return quantity.
    pub quantity: i64,
    /// Quantity received so far.
    pub received_quantity: i64,
    /// Quantity written off so far.
    pub written_off_quantity: i64,
    /// Optional line-level reason.
    pub reason: Option<String>,
    /// Line state.
    pub status: ReturnItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// A claim for one order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderClaim {
    /// Claim identifier.
    pub id: ClaimId,
    /// Owning order.
    pub order_id: OrderId,
    /// Claim lifecycle state.
    pub status: ClaimStatus,
    /// Optional reason for the claim.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Acceptance timestamp in Unix seconds.
    pub accepted_at: Option<Timestamp>,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Claimed lines.
    pub items: Vec<OrderClaimItem>,
}

/// One line of a claim.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderClaimItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning claim.
    pub claim_id: ClaimId,
    /// Historical order line affected by the claim.
    pub order_item_id: LineItemId,
    /// Claimed quantity.
    pub quantity: i64,
    /// Refund or replacement resolution.
    pub item_type: ClaimItemType,
    /// Replacement variant for `replace` claims.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
    /// Line state.
    pub status: ClaimItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// An exchange for one order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderExchange {
    /// Exchange identifier.
    pub id: ExchangeId,
    /// Owning order.
    pub order_id: OrderId,
    /// Exchange lifecycle state.
    pub status: ExchangeStatus,
    /// Optional reason for the exchange.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Acceptance timestamp in Unix seconds.
    pub accepted_at: Option<Timestamp>,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Items returned by the customer.
    pub inbound_items: Vec<OrderExchangeItem>,
    /// Replacement items shipped to the customer.
    pub outbound_items: Vec<OrderExchangeItem>,
}

/// One line of an exchange.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderExchangeItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning exchange.
    pub exchange_id: ExchangeId,
    /// Historical order line this exchange line refers to.
    pub order_item_id: LineItemId,
    /// Inbound or outbound direction.
    pub direction: ExchangeItemDirection,
    /// Exchanged quantity.
    pub quantity: i64,
    /// Replacement variant for outbound lines.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
    /// Line state.
    pub status: ExchangeItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// A proposed order edit with recalculated totals.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderEdit {
    /// Edit identifier.
    pub id: OrderEditId,
    /// Owning order.
    pub order_id: OrderId,
    /// Edit lifecycle state.
    pub status: OrderEditStatus,
    /// Optional reason for the edit.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Order subtotal before the edit.
    pub before_subtotal: i64,
    /// Order subtotal after the edit.
    pub after_subtotal: i64,
    /// Order total before the edit.
    pub before_total: i64,
    /// Order total after the edit.
    pub after_total: i64,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Confirmation timestamp in Unix seconds.
    pub confirmed_at: Option<Timestamp>,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Edit lines.
    pub items: Vec<OrderEditItem>,
}

/// One line of an order edit.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderEditItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning edit.
    pub edit_id: OrderEditId,
    /// Historical order line for remove/update lines.
    pub order_item_id: Option<LineItemId>,
    /// Kind of change.
    pub action: OrderEditAction,
    /// Product identifier snapshot.
    pub product_id: Option<ProductId>,
    /// Variant identifier snapshot.
    pub variant_id: Option<VariantId>,
    /// Product title snapshot.
    pub product_title: String,
    /// Variant title snapshot.
    pub variant_title: Option<String>,
    /// SKU snapshot.
    pub sku: Option<String>,
    /// Unit price in minor units.
    pub unit_price: i64,
    /// Quantity before the edit.
    pub before_quantity: i64,
    /// Quantity after the edit.
    pub after_quantity: i64,
    /// Line state.
    pub status: OrderEditItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for one returned line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReturnItemInput {
    /// Historical order line being returned.
    pub order_item_id: LineItemId,
    /// Requested return quantity.
    pub quantity: i64,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Input for [`crate::OrderService::create_return`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateReturnInput {
    /// Optional reason for the return.
    pub reason: Option<String>,
    /// Refund created for this return, when linked.
    pub refund_id: Option<RefundId>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Returned lines.
    pub items: Vec<ReturnItemInput>,
}

/// Input for one received return line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReceiveReturnItemInput {
    /// Return line's historical order line.
    pub order_item_id: LineItemId,
    /// Incremental quantity received.
    pub quantity: i64,
}

/// Input for [`crate::OrderService::receive_return`].
///
/// An empty `items` list receives every outstanding quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReceiveReturnInput {
    /// Lines to receive.
    pub items: Vec<ReceiveReturnItemInput>,
}

/// Input for one written-off return line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WriteOffReturnItemInput {
    /// Return line's historical order line.
    pub order_item_id: LineItemId,
    /// Incremental quantity to write off.
    pub quantity: i64,
}

/// Input for [`crate::OrderService::write_off_return`].
///
/// An empty `items` list writes off every received quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WriteOffReturnInput {
    /// Lines to write off.
    pub items: Vec<WriteOffReturnItemInput>,
}

/// Input for one claimed line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimItemInput {
    /// Historical order line affected by the claim.
    pub order_item_id: LineItemId,
    /// Claimed quantity.
    pub quantity: i64,
    /// Refund or replacement resolution.
    pub item_type: ClaimItemType,
    /// Replacement variant for `replace` claims.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Input for [`crate::OrderService::create_claim`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateClaimInput {
    /// Optional reason for the claim.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Claimed lines.
    pub items: Vec<ClaimItemInput>,
}

/// Input for one exchange line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExchangeItemInput {
    /// Historical order line this exchange line refers to.
    pub order_item_id: LineItemId,
    /// Exchanged quantity.
    pub quantity: i64,
    /// Replacement variant for outbound lines.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Input for [`crate::OrderService::create_exchange`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateExchangeInput {
    /// Optional reason for the exchange.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Items returned by the customer.
    pub inbound_items: Vec<ExchangeItemInput>,
    /// Replacement items shipped to the customer.
    pub outbound_items: Vec<ExchangeItemInput>,
}

/// Input for one order edit line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderEditItemInput {
    /// Historical order line for remove/update lines.
    pub order_item_id: Option<LineItemId>,
    /// Kind of change.
    pub action: OrderEditAction,
    /// Product identifier snapshot for added lines.
    pub product_id: Option<ProductId>,
    /// Variant identifier snapshot for added lines.
    pub variant_id: Option<VariantId>,
    /// Product title snapshot for added lines.
    pub product_title: Option<String>,
    /// Variant title snapshot for added lines.
    pub variant_title: Option<String>,
    /// SKU snapshot for added lines.
    pub sku: Option<String>,
    /// Unit price for added lines; ignored for existing lines.
    pub unit_price: Option<i64>,
    /// Expected current quantity for remove/update lines.
    pub before_quantity: Option<i64>,
    /// Quantity after the edit.
    pub after_quantity: i64,
}

/// Input for [`crate::OrderService::create_order_edit`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateOrderEditInput {
    /// Optional reason for the edit.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Edit lines.
    pub items: Vec<OrderEditItemInput>,
}

#[cfg(test)]
mod tests {
    use super::{
        ClaimItemType, ClaimStatus, ExchangeItemDirection, ExchangeStatus, OrderEditAction,
        OrderEditStatus, ReturnItemStatus, ReturnStatus,
    };

    #[test]
    fn statuses_round_trip_through_storage() {
        for status in [
            ReturnStatus::Draft,
            ReturnStatus::Requested,
            ReturnStatus::PartiallyReceived,
            ReturnStatus::Received,
            ReturnStatus::WrittenOff,
            ReturnStatus::Canceled,
        ] {
            assert_eq!(ReturnStatus::from_stored(status.as_str()).unwrap(), status);
        }
        for status in [
            ReturnItemStatus::Requested,
            ReturnItemStatus::Received,
            ReturnItemStatus::WrittenOff,
            ReturnItemStatus::Canceled,
        ] {
            assert_eq!(
                ReturnItemStatus::from_stored(status.as_str()).unwrap(),
                status
            );
        }
        for status in [
            ClaimStatus::Requested,
            ClaimStatus::Accepted,
            ClaimStatus::Canceled,
        ] {
            assert_eq!(ClaimStatus::from_stored(status.as_str()).unwrap(), status);
        }
        for status in [
            ExchangeStatus::Requested,
            ExchangeStatus::Accepted,
            ExchangeStatus::Canceled,
        ] {
            assert_eq!(
                ExchangeStatus::from_stored(status.as_str()).unwrap(),
                status
            );
        }
        for status in [
            OrderEditStatus::Draft,
            OrderEditStatus::Confirmed,
            OrderEditStatus::Canceled,
        ] {
            assert_eq!(
                OrderEditStatus::from_stored(status.as_str()).unwrap(),
                status
            );
        }
        for status in [ClaimItemType::Refund, ClaimItemType::Replace] {
            assert_eq!(ClaimItemType::from_stored(status.as_str()).unwrap(), status);
        }
        for status in [
            ExchangeItemDirection::Inbound,
            ExchangeItemDirection::Outbound,
        ] {
            assert_eq!(
                ExchangeItemDirection::from_stored(status.as_str()).unwrap(),
                status
            );
        }
        for status in [
            OrderEditAction::Add,
            OrderEditAction::Remove,
            OrderEditAction::Update,
        ] {
            assert_eq!(
                OrderEditAction::from_stored(status.as_str()).unwrap(),
                status
            );
        }
        assert!(ReturnStatus::from_stored("unknown").is_err());
    }

    #[test]
    fn statuses_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ReturnStatus::PartiallyReceived).unwrap(),
            "\"partially_received\""
        );
        assert_eq!(
            serde_json::to_string(&OrderEditAction::Remove).unwrap(),
            "\"remove\""
        );
        assert_eq!(
            serde_json::to_string(&ExchangeItemDirection::Outbound).unwrap(),
            "\"outbound\""
        );
    }

    #[test]
    fn only_early_returns_can_be_canceled() {
        assert!(ReturnStatus::Draft.is_cancelable());
        assert!(ReturnStatus::Requested.is_cancelable());
        assert!(ReturnStatus::PartiallyReceived.is_cancelable());
        assert!(!ReturnStatus::Received.is_cancelable());
        assert!(!ReturnStatus::WrittenOff.is_cancelable());
        assert!(!ReturnStatus::Canceled.is_cancelable());
    }
}
