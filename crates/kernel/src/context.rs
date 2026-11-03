//! Trusted request and store contexts.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::ids::{CustomerId, StoreId, UserId};
use crate::money::CurrencyCode;

/// The trusted context of the store serving a request.
///
/// Store context is established by the store resolver on the Rust side. Clients
/// must never be able to select an arbitrary store through a header or query
/// parameter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoreContext {
    /// Resolved store identifier.
    pub store_id: StoreId,
    /// Normalized hostname that resolved to this store.
    pub domain: String,
    /// Store locale, for example `en-US`.
    pub locale: String,
    /// Store display currency.
    pub currency: CurrencyCode,
    /// IANA time zone identifier.
    pub timezone: String,
}

/// The authenticated actor for a request.
///
/// Permissions are resolved by the authentication middleware for the selected
/// store. Application handlers must still verify permissions at the boundary:
/// frontend route protection is never sufficient.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ActorContext {
    /// Platform user identifier, for admin requests.
    pub user_id: Option<UserId>,
    /// Store customer identifier, for storefront requests.
    pub customer_id: Option<CustomerId>,
    /// API token identifier, when authenticated by token.
    pub api_token_id: Option<String>,
    /// Store the actor is acting within.
    pub store_id: Option<StoreId>,
    /// Permissions granted for the selected store.
    pub permissions: BTreeSet<String>,
    /// How the actor authenticated.
    pub authentication: AuthenticationMethod,
}

/// Authentication mechanism used by the request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationMethod {
    /// No authentication was presented.
    #[default]
    Anonymous,
    /// Server-side admin session cookie.
    Session,
    /// Bearer API token.
    ApiToken,
    /// Store-scoped customer session.
    CustomerSession,
}

impl ActorContext {
    /// Creates an empty anonymous actor.
    #[must_use]
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// Creates a platform user actor with explicit permissions.
    #[must_use]
    pub fn user(
        user_id: UserId,
        store_id: Option<StoreId>,
        permissions: impl IntoIterator<Item = String>,
        authentication: AuthenticationMethod,
    ) -> Self {
        Self {
            user_id: Some(user_id),
            customer_id: None,
            api_token_id: None,
            store_id,
            permissions: permissions.into_iter().collect(),
            authentication,
        }
    }

    /// Creates a customer actor.
    #[must_use]
    pub fn customer(customer_id: CustomerId, store_id: StoreId) -> Self {
        Self {
            user_id: None,
            customer_id: Some(customer_id),
            api_token_id: None,
            store_id: Some(store_id),
            permissions: BTreeSet::new(),
            authentication: AuthenticationMethod::CustomerSession,
        }
    }

    /// Returns true when the actor holds the permission.
    #[must_use]
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.contains(permission)
    }

    /// Requires a permission, returning a stable forbidden error otherwise.
    pub fn require_permission(&self, permission: &str) -> Result<(), AppError> {
        if self.has_permission(permission) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }

    /// Requires an authenticated platform user.
    pub fn require_user(&self) -> Result<&UserId, AppError> {
        self.user_id.as_ref().ok_or(AppError::Unauthorized)
    }

    /// Requires an authenticated customer.
    pub fn require_customer(&self) -> Result<&CustomerId, AppError> {
        self.customer_id.as_ref().ok_or(AppError::Unauthorized)
    }

    /// Returns true when authenticated by a browser cookie.
    #[must_use]
    pub fn is_cookie_authenticated(&self) -> bool {
        matches!(
            self.authentication,
            AuthenticationMethod::Session | AuthenticationMethod::CustomerSession
        )
    }
}

/// Request-scoped context propagated explicitly through the application.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RequestContext {
    /// Correlation identifier for logging and error responses.
    pub request_id: String,
    /// Resolved store, when the route is store-scoped.
    pub store: Option<StoreContext>,
    /// Authenticated actor, when applicable.
    pub actor: Option<ActorContext>,
    /// Optional request locale override negotiated by the storefront.
    pub locale: Option<String>,
}

impl RequestContext {
    /// Creates a request context with a fresh correlation identifier.
    #[must_use]
    pub fn new() -> Self {
        Self {
            request_id: uuid::Uuid::now_v7().to_string(),
            store: None,
            actor: None,
            locale: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ActorContext, AuthenticationMethod};
    use crate::ids::{StoreId, UserId};

    #[test]
    fn permissions_are_enforced() {
        let actor = ActorContext::user(
            UserId::new(),
            Some(StoreId::new()),
            ["products.read".to_owned()],
            AuthenticationMethod::Session,
        );

        assert!(actor.require_permission("products.read").is_ok());
        assert!(actor.require_permission("products.write").is_err());
        assert!(actor.is_cookie_authenticated());
    }

    #[test]
    fn anonymous_actor_is_unauthenticated() {
        let actor = ActorContext::anonymous();
        assert!(actor.require_user().is_err());
        assert!(!actor.is_cookie_authenticated());
    }
}
