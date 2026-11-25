//! Capability scopes a plugin may request.

use serde::{Deserialize, Serialize};

use crate::error::PluginError;

/// A capability granted to a plugin by its manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// Store, locale and non-secret plugin settings.
    #[serde(rename = "core.context")]
    Context,
    /// Read catalog data.
    #[serde(rename = "catalog.read")]
    CatalogRead,
    /// Read orders.
    #[serde(rename = "orders.read")]
    OrdersRead,
    /// Read customers.
    #[serde(rename = "customers.read")]
    CustomersRead,
    /// Read content pages.
    #[serde(rename = "content.read")]
    ContentRead,
    /// Read carts.
    #[serde(rename = "carts.read")]
    CartsRead,
    /// Read plugin key-value storage.
    #[serde(rename = "kv.read")]
    KvRead,
    /// Write plugin key-value storage.
    #[serde(rename = "kv.write")]
    KvWrite,
    /// Read plugin secrets through the host API.
    #[serde(rename = "secrets.read")]
    SecretsRead,
    /// Subscribe to platform events.
    #[serde(rename = "events.subscribe")]
    EventsSubscribe,
    /// Call allowlisted external hosts through the egress proxy.
    #[serde(rename = "network.egress")]
    NetworkEgress,
    /// Enqueue plugin jobs.
    #[serde(rename = "jobs.enqueue")]
    JobsEnqueue,
    /// Render admin UI extensions.
    #[serde(rename = "admin.ui")]
    AdminUi,
    /// Render storefront extensions.
    #[serde(rename = "storefront.ui")]
    StorefrontUi,
    /// Read public store data from the storefront.
    #[serde(rename = "store.read")]
    StorefrontRead,
    /// Read the signed-in storefront customer.
    #[serde(rename = "customer.read")]
    CustomerRead,
}

impl Scope {
    /// String form used in manifests and tokens.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Context => "core.context",
            Self::CatalogRead => "catalog.read",
            Self::OrdersRead => "orders.read",
            Self::CustomersRead => "customers.read",
            Self::ContentRead => "content.read",
            Self::CartsRead => "carts.read",
            Self::KvRead => "kv.read",
            Self::KvWrite => "kv.write",
            Self::SecretsRead => "secrets.read",
            Self::EventsSubscribe => "events.subscribe",
            Self::NetworkEgress => "network.egress",
            Self::JobsEnqueue => "jobs.enqueue",
            Self::AdminUi => "admin.ui",
            Self::StorefrontUi => "storefront.ui",
            Self::StorefrontRead => "store.read",
            Self::CustomerRead => "customer.read",
        }
    }

    /// Parses a scope name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "core.context" => Self::Context,
            "catalog.read" => Self::CatalogRead,
            "orders.read" => Self::OrdersRead,
            "customers.read" => Self::CustomersRead,
            "content.read" => Self::ContentRead,
            "carts.read" => Self::CartsRead,
            "kv.read" => Self::KvRead,
            "kv.write" => Self::KvWrite,
            "secrets.read" => Self::SecretsRead,
            "events.subscribe" => Self::EventsSubscribe,
            "network.egress" => Self::NetworkEgress,
            "jobs.enqueue" => Self::JobsEnqueue,
            "admin.ui" => Self::AdminUi,
            "storefront.ui" => Self::StorefrontUi,
            "store.read" => Self::StorefrontRead,
            "customer.read" => Self::CustomerRead,
            _ => return None,
        })
    }

    /// Returns true for scopes that mutate business data.
    #[must_use]
    pub const fn is_write(self) -> bool {
        matches!(self, Self::KvWrite)
    }
}

/// Write scopes reserved for a later host API version.
pub const RESERVED_WRITE_SCOPES: &[&str] = &[
    "catalog.write",
    "orders.write",
    "customers.write",
    "inventory.write",
    "promotions.write",
    "settings.write",
    "themes.write",
    "payments.write",
    "secrets.write",
];

/// Parses a scope declared in one capability group.
///
/// # Errors
///
/// Returns [`PluginError::UnsupportedScope`] for unknown names, reserved write
/// scopes, or a scope used in the wrong group.
pub fn parse_scope(group: &str, value: &str) -> Result<Scope, PluginError> {
    if RESERVED_WRITE_SCOPES.contains(&value) {
        return Err(PluginError::UnsupportedScope(format!(
            "{value} is reserved for a later host API version"
        )));
    }
    let scope =
        Scope::parse(value).ok_or_else(|| PluginError::UnsupportedScope(value.to_owned()))?;
    let allowed = match group {
        "data" => matches!(
            scope,
            Scope::CatalogRead
                | Scope::OrdersRead
                | Scope::CustomersRead
                | Scope::ContentRead
                | Scope::CartsRead
        ),
        "storage" => matches!(scope, Scope::KvRead | Scope::KvWrite),
        "secrets" => matches!(scope, Scope::SecretsRead),
        "storefront" => matches!(scope, Scope::StorefrontRead | Scope::CustomerRead),
        _ => false,
    };
    if !allowed {
        return Err(PluginError::UnsupportedScope(format!(
            "{value} is not valid in the {group} group"
        )));
    }
    Ok(scope)
}
