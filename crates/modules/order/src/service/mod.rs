//! SQLite-backed order application service.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::{AppError, EventId, OrderId, Timestamp};

use crate::domain::{OrderStatus, OrderStatusHistory};

mod creation;
mod drafts;
mod fulfillment;
mod reads;
mod refunds;
mod status;

/// Scope used for order-creation idempotency keys.
const IDEMPOTENCY_SCOPE: &str = "order.create";

/// Scope used for idempotent order-refund keys.
const REFUND_IDEMPOTENCY_SCOPE: &str = "order.refund";

/// Order application service.
#[derive(Clone, Debug)]
pub struct OrderService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl OrderService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records mutating order operations in the store audit outbox.
    ///
    /// When an actor is set, every audited mutation appends its audit entry in
    /// the same transaction as the change.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Returns a copy of this service that does not record audit entries.
    ///
    /// Workflows use this for internally implied transitions whose audit is
    /// owned by a different step, avoiding duplicate entries.
    #[must_use]
    pub fn without_audit(mut self) -> Self {
        self.audit = None;
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    pub(crate) async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
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

    /// Returns the store database backing this service.
    ///
    /// Used by sibling application modules (post-purchase flows) that extend
    /// the order service without duplicating persistence plumbing.
    #[must_use]
    pub(crate) fn database(&self) -> &StoreDatabase {
        &self.db
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
}

fn json_text(value: &impl serde::Serialize) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::validation(format!("invalid JSON value: {error}")))
}

pub(crate) async fn insert_history(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    order_id: &OrderId,
    status: OrderStatus,
    previous_status: Option<OrderStatus>,
    note: Option<&str>,
    created_at: Timestamp,
) -> Result<OrderStatusHistory, AppError> {
    let id = EventId::new();
    sqlx::query(
        "INSERT INTO order_status_history
            (id, order_id, status, previous_status, note, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id.as_str())
    .bind(order_id.as_str())
    .bind(status.as_str())
    .bind(previous_status.map(OrderStatus::as_str))
    .bind(note)
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(OrderStatusHistory {
        id: id.as_str().to_owned(),
        order_id: order_id.clone(),
        status,
        previous_status,
        note: note.map(str::to_owned),
        created_at,
    })
}

#[cfg(test)]
mod draft_tests;
#[cfg(test)]
mod line_discount_tests;
#[cfg(test)]
mod tests;
