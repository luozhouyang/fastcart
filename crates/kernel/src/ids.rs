//! Strongly typed entity identifiers.
//!
//! Each identifier is a distinct newtype so values cannot be mixed across entity
//! types. The storage format is `"{prefix}_{uuid}"` using UUID v7 for roughly
//! time-ordered identifiers.

macro_rules! define_id {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            serde::Serialize,
            serde::Deserialize,
            utoipa::ToSchema,
        )]
        #[serde(transparent)]
        #[schema(value_type = String)]
        pub struct $name(String);

        impl $name {
            /// Creates a new identifier with a UUID v7 suffix.
            #[must_use]
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, uuid::Uuid::now_v7().simple()))
            }

            /// Wraps an existing identifier string without validation.
            #[must_use]
            pub fn from_string(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the identifier as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

define_id!(StoreId, "store", "Identifier of a store.");
define_id!(UserId, "user", "Identifier of a platform user.");
define_id!(CustomerId, "cust", "Identifier of a customer.");
define_id!(
    CustomerSessionId,
    "csess",
    "Identifier of a customer session."
);
define_id!(ProductId, "prod", "Identifier of a product.");
define_id!(VariantId, "var", "Identifier of a product variant.");
define_id!(OptionId, "opt", "Identifier of a product option.");
define_id!(
    OptionValueId,
    "optval",
    "Identifier of a product option value."
);
define_id!(ImageId, "img", "Identifier of a product image.");
define_id!(CategoryId, "cat", "Identifier of a category.");
define_id!(CollectionId, "coll", "Identifier of a collection.");
define_id!(TagId, "tag", "Identifier of a tag.");
define_id!(PriceId, "price", "Identifier of a price.");
define_id!(PriceListId, "plist", "Identifier of a price list.");
define_id!(PriceSetId, "pset", "Identifier of a price set.");
define_id!(CartId, "cart", "Identifier of a cart.");
define_id!(
    LineItemId,
    "item",
    "Identifier of a cart or order line item."
);
define_id!(CheckoutId, "chk", "Identifier of a checkout session.");
define_id!(OrderId, "order", "Identifier of an order.");
define_id!(FulfillmentId, "ful", "Identifier of a fulfillment.");
define_id!(RefundId, "refund", "Identifier of a refund.");
define_id!(ReturnId, "ret", "Identifier of an order return.");
define_id!(ClaimId, "claim", "Identifier of an order claim.");
define_id!(ExchangeId, "exch", "Identifier of an order exchange.");
define_id!(OrderEditId, "oedit", "Identifier of an order edit.");
define_id!(PaymentId, "pay", "Identifier of a payment.");
define_id!(
    PaymentSessionId,
    "paysess",
    "Identifier of a payment session."
);
define_id!(
    PaymentAttemptId,
    "paytry",
    "Identifier of a payment attempt."
);
define_id!(PromotionId, "promo", "Identifier of a promotion.");
define_id!(CouponId, "coupon", "Identifier of a coupon.");
define_id!(
    ShippingProfileId,
    "shprof",
    "Identifier of a shipping profile."
);
define_id!(
    ShippingOptionId,
    "shopt",
    "Identifier of a shipping option."
);
define_id!(TaxRegionId, "taxreg", "Identifier of a tax region.");
define_id!(TaxRateId, "taxrate", "Identifier of a tax rate.");
define_id!(TaxCategoryId, "taxcat", "Identifier of a tax category.");
define_id!(InventoryItemId, "iitem", "Identifier of an inventory item.");
define_id!(
    InventoryLocationId,
    "iloc",
    "Identifier of an inventory location."
);
define_id!(
    ReservationId,
    "res",
    "Identifier of an inventory reservation."
);
define_id!(PageId, "page", "Identifier of a content page.");
define_id!(NavigationId, "nav", "Identifier of a navigation menu.");
define_id!(ThemeId, "theme", "Identifier of a storefront theme.");
define_id!(
    ThemeActivationId,
    "thact",
    "Identifier of a theme activation."
);
define_id!(SessionId, "sess", "Identifier of an admin session.");
define_id!(ApiTokenId, "token", "Identifier of an API token.");
define_id!(JobId, "job", "Identifier of a background job.");
define_id!(EventId, "evt", "Identifier of a domain event.");
define_id!(WebhookId, "whook", "Identifier of a webhook endpoint.");
define_id!(
    WebhookDeliveryId,
    "whdel",
    "Identifier of a webhook delivery attempt."
);

#[cfg(test)]
mod tests {
    use super::{OrderId, StoreId};

    #[test]
    fn ids_are_prefixed_and_time_ordered() {
        let first = StoreId::new();
        let second = StoreId::new();

        assert!(first.as_str().starts_with("store_"));
        assert_ne!(first, second);
        assert!(first.as_str() < second.as_str());
    }

    #[test]
    fn ids_are_distinct_types() {
        let store = StoreId::new();
        let order = OrderId::new();

        assert!(store.as_str().starts_with("store_"));
        assert!(order.as_str().starts_with("order_"));
    }
}
