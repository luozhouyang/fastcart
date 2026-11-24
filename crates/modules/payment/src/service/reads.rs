//! Payment, attempt and refund reads.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, OrderId, PageResult};
use sqlx::{QueryBuilder, Sqlite};

use crate::domain::{ListPaymentsFilter, Payment, PaymentAttempt};

use super::store::*;
use super::*;
impl PaymentService {
    /// Loads a payment.
    pub async fn get_payment(&self, payment_id: &str) -> Result<Payment, AppError> {
        let row = sqlx::query_as::<_, PaymentRow>(
            "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
             status, captured_at, metadata, created_at, updated_at \
             FROM payments WHERE id = ?",
        )
        .bind(payment_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(payment_from_row)
            .ok_or_else(|| AppError::not_found("payment", payment_id))
    }

    /// Lists payments with optional order/status filters and pagination.
    pub async fn list_payments(
        &self,
        filter: ListPaymentsFilter,
    ) -> Result<PageResult<Payment>, AppError> {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
             status, captured_at, metadata, created_at, updated_at FROM payments WHERE 1 = 1",
        );
        let mut count_query =
            QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM payments WHERE 1 = 1");
        if let Some(order_id) = filter.order_id.as_ref() {
            query.push(" AND order_id = ").push_bind(order_id.as_str());
            count_query
                .push(" AND order_id = ")
                .push_bind(order_id.as_str());
        }
        if let Some(status) = filter.status {
            query.push(" AND status = ").push_bind(status.as_str());
            count_query
                .push(" AND status = ")
                .push_bind(status.as_str());
        }
        query
            .push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(filter.pagination.limit)
            .push(" OFFSET ")
            .push_bind(filter.pagination.offset);
        let rows: Vec<PaymentRow> = query
            .build_query_as()
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let count: i64 = count_query
            .build_query_scalar()
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(PageResult::new(
            rows.into_iter().map(payment_from_row).collect(),
            count,
            filter.pagination,
        ))
    }

    /// Lists attempts for a payment session.
    pub async fn list_attempts(&self, session_id: &str) -> Result<Vec<PaymentAttempt>, AppError> {
        let rows = sqlx::query_as::<_, PaymentAttemptRow>(
            "SELECT id, payment_session_id, amount, currency_code, status, \
             provider_reference, error, created_at \
             FROM payment_attempts WHERE payment_session_id = ? \
             ORDER BY created_at ASC, id ASC",
        )
        .bind(session_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows.into_iter().map(attempt_from_row).collect())
    }

    /// Total actually captured for an order.
    ///
    /// A `payments` row is written once per capture, so the sum of its amounts
    /// is the money the provider took. Only captured states are counted — a
    /// refunded row still records money that was captured — and refunded money
    /// is reported separately rather than subtracted here.
    pub async fn captured_total_for_order(&self, order_id: &OrderId) -> Result<i64, AppError> {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM payments \
             WHERE order_id = ? AND status IN \
                   ('captured', 'partially_captured', 'partially_refunded', 'refunded')",
        )
        .bind(order_id.as_str())
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)
    }

    /// Total refunded for an order, across every capture record.
    ///
    /// Pending refunds count: the refund row is written before the provider is
    /// asked, and a refund that later fails is moved to `failed` by the
    /// webhook, which takes it back out of this total.
    pub async fn refunded_total_for_order(&self, order_id: &OrderId) -> Result<i64, AppError> {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount), 0) FROM refunds \
             WHERE order_id = ? AND status != 'failed'",
        )
        .bind(order_id.as_str())
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)
    }
}
