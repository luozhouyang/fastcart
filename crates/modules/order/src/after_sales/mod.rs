//! Post-purchase domain entities: returns, claims, exchanges and order edits.
//!
//! Returns, claims and exchanges never rewrite the `order_items` rows they
//! reference: requested, received and written-off quantities and replacement
//! variants are persisted in dedicated tables and exposed through the order
//! aggregate. A confirmed order edit is the one flow that lands on the lines:
//! it inserts added lines, rewrites changed quantities and amounts, and zeroes
//! removed ones, while `order_edits` and `order_edit_items` record what did
//! change.

use fastcart_kernel::{
    ClaimId, EventId, ExchangeId, FulfillmentId, LineItemId, Metadata, OrderEditId, OrderId,
    ProductId, RefundId, ReturnId, Timestamp, VariantId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

    /// Returns true when the return has not reached a terminal state.
    ///
    /// Open returns still owe the customer something: they may be submitted,
    /// received, refunded or written off. A `canceled` or `written_off` return
    /// is finished and no longer blocks a new request for the same order.
    #[must_use]
    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::Draft | Self::Requested | Self::PartiallyReceived | Self::Received
        )
    }
}

string_enum! {
    /// Who created a return request.
    pub enum ReturnSource {
        /// Created by store staff through the admin API.
        Admin => "admin",
        /// Requested by the customer through the storefront.
        Customer => "customer",
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
        /// Edit applied: the line changes are materialized and the order totals recalculated.
        Confirmed => "confirmed",
        /// Edit canceled without changing the order.
        Canceled => "canceled",
    }
}

string_enum! {
    /// Kind of change proposed by an order edit line.
    pub enum OrderEditAction {
        /// Add a line the original purchase did not contain.
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

mod claims;
mod edits;
mod exchanges;
mod fulfillments;
mod return_reasons;
mod returns;

pub use claims::{ClaimItemInput, CreateClaimInput, OrderClaim, OrderClaimItem};
pub use edits::{CreateOrderEditInput, OrderEdit, OrderEditItem, OrderEditItemInput};
pub use exchanges::{
    CreateExchangeInput, ExchangeItemInput, OrderExchange, OrderExchangeItem, ReplacementUnitPrice,
};
pub use fulfillments::{
    AfterSalesFulfillment, AfterSalesFulfillmentItem, AfterSalesFulfillmentItemInput,
    CreateAfterSalesFulfillmentInput,
};
pub use return_reasons::{
    CreateReturnReasonInput, ReturnReason, ReturnReasonId, UpdateReturnReasonInput,
};
pub use returns::{
    CreateReturnInput, OrderReturn, OrderReturnItem, ReceiveReturnInput, ReceiveReturnItemInput,
    ReturnItemInput, StoreReturnFilter, StoreReturnRow, WriteOffReturnInput,
    WriteOffReturnItemInput,
};

#[cfg(test)]
mod tests {
    use super::{
        ClaimItemType, ClaimStatus, ExchangeItemDirection, ExchangeStatus, OrderEditAction,
        OrderEditStatus, ReturnItemStatus, ReturnSource, ReturnStatus,
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

    #[test]
    fn only_unfinished_returns_are_open() {
        assert!(ReturnStatus::Draft.is_open());
        assert!(ReturnStatus::Requested.is_open());
        assert!(ReturnStatus::PartiallyReceived.is_open());
        assert!(ReturnStatus::Received.is_open());
        assert!(!ReturnStatus::WrittenOff.is_open());
        assert!(!ReturnStatus::Canceled.is_open());
    }

    #[test]
    fn return_sources_round_trip_through_storage() {
        assert_eq!(
            ReturnSource::from_stored("admin").unwrap(),
            ReturnSource::Admin
        );
        assert_eq!(
            ReturnSource::from_stored("customer").unwrap(),
            ReturnSource::Customer
        );
        assert!(ReturnSource::from_stored("unknown").is_err());
        assert_eq!(ReturnSource::Admin.as_str(), "admin");
        assert_eq!(ReturnSource::Customer.as_str(), "customer");
    }
}
