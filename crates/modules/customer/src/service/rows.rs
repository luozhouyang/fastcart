//! Shared persistence rows for customers and sessions.

//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use fastcart_kernel::{CustomerId, Metadata};
use sqlx::FromRow;

use crate::domain::{Customer, CustomerStatus};

#[derive(Debug, FromRow)]
pub(super) struct CustomerRow {
    pub(super) id: String,
    pub(super) email: String,
    pub(super) first_name: Option<String>,
    pub(super) last_name: Option<String>,
    pub(super) phone: Option<String>,
    pub(super) has_account: i64,
    pub(super) status: String,
    pub(super) email_verified_at: Option<i64>,
    pub(super) metadata: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) deleted_at: Option<i64>,
}

impl CustomerRow {
    pub(super) fn into_customer(self) -> Customer {
        Customer {
            id: CustomerId::from_string(self.id),
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            phone: self.phone,
            has_account: self.has_account != 0,
            status: CustomerStatus::from_db(&self.status),
            email_verified_at: self.email_verified_at,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }
}

#[derive(Debug, FromRow)]
pub(super) struct ExistingCustomerRow {
    pub(super) id: String,
    pub(super) has_account: i64,
    pub(super) deleted_at: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(super) struct CredentialCustomerRow {
    pub(super) id: String,
    pub(super) email: String,
    pub(super) first_name: Option<String>,
    pub(super) last_name: Option<String>,
    pub(super) phone: Option<String>,
    pub(super) has_account: i64,
    pub(super) status: String,
    pub(super) email_verified_at: Option<i64>,
    pub(super) metadata: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) password_hash: String,
}

impl CredentialCustomerRow {
    pub(super) fn into_customer(self) -> Customer {
        Customer {
            id: CustomerId::from_string(self.id),
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            phone: self.phone,
            has_account: self.has_account != 0,
            status: CustomerStatus::from_db(&self.status),
            email_verified_at: self.email_verified_at,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: None,
        }
    }
}

#[derive(Debug, FromRow)]
pub(super) struct SessionCustomerRow {
    pub(super) session_id: String,
    pub(super) expires_at: i64,
    pub(super) id: String,
    pub(super) email: String,
    pub(super) first_name: Option<String>,
    pub(super) last_name: Option<String>,
    pub(super) phone: Option<String>,
    pub(super) has_account: i64,
    pub(super) status: String,
    pub(super) email_verified_at: Option<i64>,
    pub(super) metadata: String,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
}

impl SessionCustomerRow {
    pub(super) fn into_customer(self) -> Customer {
        Customer {
            id: CustomerId::from_string(self.id),
            email: self.email,
            first_name: self.first_name,
            last_name: self.last_name,
            phone: self.phone,
            has_account: self.has_account != 0,
            status: CustomerStatus::from_db(&self.status),
            email_verified_at: self.email_verified_at,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: None,
        }
    }
}
