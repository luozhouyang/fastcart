//! SQLite-backed promotion service.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::AppError;
use sqlx::Sqlite;

const PROMOTION_COLUMNS: &str = "id, code, title, type AS promotion_type, value, status, \
     is_automatic, starts_at, ends_at, usage_limit, usage_count, per_customer_limit, priority, \
     is_stackable, target, metadata, created_at, updated_at";
const PROMOTION_RULE_COLUMNS: &str = "id, promotion_id, attribute, operator, value, created_at";
const COUPON_COLUMNS: &str = "id, code, promotion_id, usage_limit, usage_count, starts_at, \
     ends_at, metadata, created_at, updated_at";

/// Maximum number of bound parameters used in one `IN (...)` statement.
/// SQLite's default limit is 999; staying under it leaves headroom for the
/// other parameters a query may carry.
const SQLITE_PARAMETER_LIMIT: usize = 900;

/// Prefix of a checkout-held promotion usage key.
const CHECKOUT_USAGE_KEY_PREFIX: &str = "checkout:";

/// Application service for promotions, coupons, usage accounting and pricing
/// calculations.
mod calculation;
mod campaigns;
mod coupons;
mod promotions;
mod usage;

#[derive(Clone, Debug)]
pub struct PromotionService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
    /// Number of `promotion_rules` reads this service has issued.
    ///
    /// Test-only instrumentation, compiled out of production builds. It exists
    /// so the batching guarantees can be asserted without a global tracing
    /// subscriber or thread bookkeeping: the counter lives on the instance
    /// under test and is incremented exactly where a rule query is executed.
    #[cfg(test)]
    rule_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl PromotionService {
    /// Creates a service over an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            db,
            audit: None,
            #[cfg(test)]
            rule_reads: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Number of `promotion_rules` reads this service has issued so far.
    #[cfg(test)]
    fn rule_reads(&self) -> usize {
        self.rule_reads.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Records campaign mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating campaign method appends its audit
    /// entry in the same transaction as the change.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database) => database.is_unique_violation(),
        _ => false,
    }
}

fn sql_markers(count: usize) -> String {
    vec!["?"; count].join(", ")
}

fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    if is_unique_violation(&error) {
        AppError::conflict(conflict_message)
    } else {
        database_error(error)
    }
}

/// Builds the usage key a checkout claims its promotions under.
///
/// [`PromotionService::reserve_usage`] records this key,
/// [`PromotionService::reassign_usage`] moves it onto the order and
/// [`PromotionService::release_usage`] gives it back, so every caller must
/// build it here instead of formatting the prefix itself.
#[must_use]
pub fn checkout_usage_key(checkout_id: &str) -> String {
    format!("{CHECKOUT_USAGE_KEY_PREFIX}{}", checkout_id.trim())
}

/// Recovers the checkout id a usage key was built from.
fn checkout_usage_key_strip(key: &str) -> &str {
    key.strip_prefix(CHECKOUT_USAGE_KEY_PREFIX).unwrap_or(key)
}

#[cfg(test)]
mod tests;
