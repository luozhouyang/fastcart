//! Customer domain entities, inputs and pure validation rules.
//!
//! The types in this module are persistence-agnostic. Sensitive values such as
//! password hashes and session tokens never appear here: credentials are only
//! handled by the application service, which keeps them out of every public
//! struct and `Debug` representation.

use std::fmt;

use fastcart_kernel::{CountryCode, CustomerId, CustomerSessionId, Metadata, Timestamp};
use serde::{Deserialize, Serialize};

/// Maximum length of a customer name component.
pub const MAX_NAME_LENGTH: usize = 120;
/// Maximum length of a phone number.
pub const MAX_PHONE_LENGTH: usize = 40;
/// Minimum accepted password length.
pub const MIN_PASSWORD_LENGTH: usize = 8;
/// Customer session lifetime in seconds (30 days).
pub const CUSTOMER_SESSION_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Lifecycle status of a customer account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomerStatus {
    /// The customer may authenticate and place orders.
    Active,
    /// The customer is disabled and cannot authenticate.
    Disabled,
}

impl CustomerStatus {
    /// Returns the database representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    /// Decodes a stored status, defaulting to [`CustomerStatus::Active`].
    #[must_use]
    pub fn from_db(value: &str) -> Self {
        match value {
            "disabled" => Self::Disabled,
            _ => Self::Active,
        }
    }
}

/// A store-scoped customer profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Customer {
    /// Customer identifier.
    pub id: CustomerId,
    /// Normalized (lowercase) email address.
    pub email: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Whether the profile has registered credentials.
    pub has_account: bool,
    /// Lifecycle status.
    pub status: CustomerStatus,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp (Unix seconds).
    pub created_at: Timestamp,
    /// Last update timestamp (Unix seconds).
    pub updated_at: Timestamp,
    /// Soft-deletion timestamp (Unix seconds).
    pub deleted_at: Option<Timestamp>,
}

/// A customer postal address.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Address {
    /// Address identifier.
    pub id: String,
    /// Owning customer.
    pub customer_id: CustomerId,
    /// Whether this is the default shipping address.
    pub is_default_shipping: bool,
    /// Whether this is the default billing address.
    pub is_default_billing: bool,
    /// Recipient given name.
    pub first_name: Option<String>,
    /// Recipient family name.
    pub last_name: Option<String>,
    /// Company name.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City or locality.
    pub city: Option<String>,
    /// Province, region or state.
    pub province: Option<String>,
    /// Postal or ZIP code.
    pub postal_code: Option<String>,
    /// Validated ISO 3166-1 country code.
    pub country_code: Option<CountryCode>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp (Unix seconds).
    pub created_at: Timestamp,
    /// Last update timestamp (Unix seconds).
    pub updated_at: Timestamp,
}

/// A customer segment used for pricing and promotions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomerGroup {
    /// Group identifier.
    pub id: String,
    /// Unique human-readable name.
    pub name: String,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp (Unix seconds).
    pub created_at: Timestamp,
    /// Last update timestamp (Unix seconds).
    pub updated_at: Timestamp,
}

/// A server-side customer session record.
///
/// The plaintext token is never stored; only its SHA-256 digest is persisted
/// and it is never exposed through this type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomerSession {
    /// Session identifier.
    pub id: CustomerSessionId,
    /// Owning customer.
    pub customer_id: CustomerId,
    /// Expiry timestamp (Unix seconds).
    pub expires_at: Timestamp,
    /// Revocation timestamp (Unix seconds).
    pub revoked_at: Option<Timestamp>,
    /// Last authentication timestamp (Unix seconds).
    pub last_used_at: Timestamp,
    /// Creation timestamp (Unix seconds).
    pub created_at: Timestamp,
}

/// A newly created customer session. The plaintext token is returned exactly
/// once and is redacted from `Debug` output.
#[derive(Clone, Serialize, Deserialize)]
pub struct CreatedCustomerSession {
    /// Opaque bearer token.
    pub token: String,
    /// Double-submit CSRF token.
    pub csrf_token: String,
    /// Session identifier.
    pub session_id: CustomerSessionId,
    /// Authenticated customer.
    pub customer: Customer,
    /// Expiry timestamp (Unix seconds).
    pub expires_at: Timestamp,
}

impl fmt::Debug for CreatedCustomerSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreatedCustomerSession")
            .field("token", &"[redacted]")
            .field("csrf_token", &"[redacted]")
            .field("session_id", &self.session_id)
            .field("customer", &self.customer)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// The result of validating a customer session token.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthenticatedCustomerSession {
    /// Authenticated customer.
    pub customer: Customer,
    /// Identifier of the session that authenticated the request.
    pub session_id: CustomerSessionId,
}

/// Input for creating a guest or administrative customer profile.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct CreateCustomerInput {
    /// Email address; normalized to lowercase before storage.
    pub email: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Free-form extension metadata.
    pub metadata: Metadata,
}

impl CreateCustomerInput {
    /// Creates a customer input with only an email address.
    #[must_use]
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            ..Self::default()
        }
    }

    /// Sets the customer name.
    #[must_use]
    pub fn with_name(
        mut self,
        first_name: impl Into<String>,
        last_name: impl Into<String>,
    ) -> Self {
        self.first_name = Some(first_name.into());
        self.last_name = Some(last_name.into());
        self
    }

    /// Sets the contact phone number.
    #[must_use]
    pub fn with_phone(mut self, phone: impl Into<String>) -> Self {
        self.phone = Some(phone.into());
        self
    }
}

/// Partial update for a customer profile.
///
/// `None` fields are left unchanged.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct UpdateCustomerInput {
    /// Replacement email address.
    pub email: Option<String>,
    /// Replacement given name.
    pub first_name: Option<String>,
    /// Replacement family name.
    pub last_name: Option<String>,
    /// Replacement phone number.
    pub phone: Option<String>,
    /// Replacement lifecycle status.
    pub status: Option<CustomerStatus>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Input for creating or replacing a customer address.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct AddressInput {
    /// Mark the address as the default shipping address.
    pub is_default_shipping: bool,
    /// Mark the address as the default billing address.
    pub is_default_billing: bool,
    /// Recipient given name.
    pub first_name: Option<String>,
    /// Recipient family name.
    pub last_name: Option<String>,
    /// Company name.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City or locality.
    pub city: Option<String>,
    /// Province, region or state.
    pub province: Option<String>,
    /// Postal or ZIP code.
    pub postal_code: Option<String>,
    /// Two-letter country code.
    pub country_code: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Free-form extension metadata.
    pub metadata: Metadata,
}

impl AddressInput {
    /// Creates a complete address input from the required fields.
    #[must_use]
    pub fn new(
        address_1: impl Into<String>,
        city: impl Into<String>,
        postal_code: impl Into<String>,
        country_code: impl Into<String>,
    ) -> Self {
        Self {
            address_1: Some(address_1.into()),
            city: Some(city.into()),
            postal_code: Some(postal_code.into()),
            country_code: Some(country_code.into()),
            ..Self::default()
        }
    }
}

/// Input for registering a customer account with email and password.
#[derive(Clone, Deserialize)]
pub struct RegisterCustomerInput {
    /// Email address; normalized to lowercase before storage.
    pub email: String,
    /// Plaintext password, hashed with Argon2id before storage.
    pub password: String,
    /// Given name.
    #[serde(default)]
    pub first_name: Option<String>,
    /// Family name.
    #[serde(default)]
    pub last_name: Option<String>,
    /// Contact phone number.
    #[serde(default)]
    pub phone: Option<String>,
}

impl RegisterCustomerInput {
    /// Creates a registration input with only credentials.
    #[must_use]
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
            first_name: None,
            last_name: None,
            phone: None,
        }
    }

    /// Sets the customer name.
    #[must_use]
    pub fn with_name(
        mut self,
        first_name: impl Into<String>,
        last_name: impl Into<String>,
    ) -> Self {
        self.first_name = Some(first_name.into());
        self.last_name = Some(last_name.into());
        self
    }
}

impl fmt::Debug for RegisterCustomerInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisterCustomerInput")
            .field("email", &self.email)
            .field("password", &"[redacted]")
            .field("first_name", &self.first_name)
            .field("last_name", &self.last_name)
            .field("phone", &self.phone)
            .finish()
    }
}

/// Input for authenticating with email and password.
#[derive(Clone, Deserialize)]
pub struct LoginInput {
    /// Email address.
    pub email: String,
    /// Plaintext password.
    pub password: String,
}

impl LoginInput {
    /// Creates a login input.
    #[must_use]
    pub fn new(email: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            password: password.into(),
        }
    }
}

impl fmt::Debug for LoginInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginInput")
            .field("email", &self.email)
            .field("password", &"[redacted]")
            .finish()
    }
}
