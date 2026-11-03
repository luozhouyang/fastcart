//! Strongly typed entity identifiers.
//!
//! Each identifier is a distinct newtype so values cannot be mixed across entity
//! types. The storage format is `"{prefix}_{uuid}"` using UUID v7 for roughly
//! time-ordered identifiers.

macro_rules! define_id {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(
            Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates a new identifier with a UUID v7 suffix.
            #[must_use]
            pub fn new() -> Self {
                Self(format!(
                    "{}_{}",
                    $prefix,
                    uuid::Uuid::now_v7().simple()
                ))
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
define_id!(ProductId, "prod", "Identifier of a product.");
define_id!(VariantId, "var", "Identifier of a product variant.");
define_id!(OrderId, "order", "Identifier of an order.");
define_id!(CustomerId, "cust", "Identifier of a customer.");
define_id!(CartId, "cart", "Identifier of a cart.");
define_id!(PaymentId, "pay", "Identifier of a payment.");

#[cfg(test)]
mod tests {
    use super::StoreId;

    #[test]
    fn ids_are_prefixed_and_time_ordered() {
        let first = StoreId::new();
        let second = StoreId::new();

        assert!(first.as_str().starts_with("store_"));
        assert_ne!(first, second);
        assert!(first.as_str() < second.as_str());
    }
}
