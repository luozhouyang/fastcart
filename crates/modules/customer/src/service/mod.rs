//! SQLite-backed customer application service.
//!
//! Every operation is implicitly scoped to the store owning the injected
//! [`StoreDatabase`]. Credentials are hashed with Argon2id and session tokens
//! are looked up exclusively by their SHA-256 digest.

use argon2::Argon2;
use argon2::password_hash::{
    PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng,
};
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::AppError;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::domain::MIN_PASSWORD_LENGTH;

const CUSTOMER_COLUMNS: &str = "id, email, first_name, last_name, phone, has_account, status, \
     email_verified_at, metadata, created_at, updated_at, deleted_at";

const ADDRESS_COLUMNS: &str = "id, customer_id, is_default_shipping, is_default_billing, \
     first_name, last_name, company, address_1, address_2, city, province, postal_code, \
     country_code, phone, metadata, created_at, updated_at";

const GROUP_COLUMNS: &str = "id, name, metadata, created_at, updated_at";

mod addresses;
mod auth;
mod customers;
mod groups;
mod rows;
mod verification;

pub struct CustomerService {
    db: StoreDatabase,
}

impl CustomerService {
    /// Creates a service bound to a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    /// Returns the underlying store database handle.
    #[must_use]
    pub fn database(&self) -> &StoreDatabase {
        &self.db
    }
}

fn clean_optional(
    value: Option<String>,
    field: &str,
    maximum: usize,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > maximum {
        return Err(AppError::validation(format!(
            "{field} must not exceed {maximum} characters"
        )));
    }
    Ok(Some(trimmed.to_owned()))
}

fn validate_new_password(password: &str) -> Result<(), AppError> {
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(AppError::validation(format!(
            "password must contain at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| AppError::internal(error.to_string()))
}

fn verify_password(stored_hash: &str, password: &str) -> bool {
    PasswordHash::new(stored_hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

/// Hashes a password on the blocking pool.
///
/// Argon2 deliberately costs tens of milliseconds; running it inline on a
/// runtime worker (login is a public, unauthenticated endpoint) starves every
/// other task scheduled on that worker.
async fn hash_password_offloaded(password: &str) -> Result<String, AppError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(password_task_error)?
}

/// Verifies a password on the blocking pool.
///
/// Only the wrong-password case is `Ok(false)`; a failed or panicked task
/// surfaces as an error so callers never report it as `Unauthorized`.
async fn verify_password_offloaded(stored_hash: &str, password: &str) -> Result<bool, AppError> {
    let stored_hash = stored_hash.to_owned();
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || verify_password(&stored_hash, &password))
        .await
        .map_err(password_task_error)
}

/// Maps a password task that failed to join into an internal error.
///
/// The task panicked or the runtime shut down, so the result says nothing
/// about the password and must not be reported as invalid credentials.
fn password_task_error(error: tokio::task::JoinError) -> AppError {
    AppError::internal(format!("password task failed: {error}"))
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    hex::encode(buffer)
}

/// SHA-256 digest used for session token lookup.
fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Generic error for missing, expired, used or wrong-type auth tokens.
fn invalid_auth_token() -> AppError {
    AppError::validation("invalid or expired token")
}

fn conflict_or_database(error: sqlx::Error, message: &'static str) -> AppError {
    if let sqlx::Error::Database(database) = &error
        && database.is_unique_violation()
    {
        return AppError::conflict(message);
    }
    database_error(error)
}

#[cfg(test)]
mod tests;
