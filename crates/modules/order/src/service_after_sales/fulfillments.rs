//! Provider-backed after-sales replacement fulfillments.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, ClaimId, EventId, ExchangeId, FulfillmentId, OrderId, VariantId, now_timestamp,
    validate_length, validate_metadata, validate_quantity,
};

use crate::after_sales::{
    AfterSalesFulfillment, AfterSalesFulfillmentItem, CreateAfterSalesFulfillmentInput,
};
use crate::service::OrderService;

use super::append_after_sales_event;

#[derive(sqlx::FromRow)]
struct AfterSalesFulfillmentRow {
    id: String,
    order_id: String,
    claim_id: Option<String>,
    exchange_id: Option<String>,
    status: String,
    provider_id: String,
    provider_fulfillment_id: Option<String>,
    provider_data: String,
    tracking_numbers: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(sqlx::FromRow)]
struct AfterSalesFulfillmentItemRow {
    id: String,
    fulfillment_id: String,
    variant_id: String,
    quantity: i64,
    created_at: i64,
}

impl AfterSalesFulfillmentItemRow {
    fn into_domain(self) -> AfterSalesFulfillmentItem {
        AfterSalesFulfillmentItem {
            id: EventId::from_string(self.id),
            fulfillment_id: FulfillmentId::from_string(self.fulfillment_id),
            variant_id: VariantId::from_string(self.variant_id),
            quantity: self.quantity,
            created_at: self.created_at,
        }
    }
}
impl OrderService {
    /// Persists a provider-backed replacement fulfillment for a claim or
    /// exchange.
    ///
    /// Exactly one of `claim_id` and `exchange_id` must be set, and the
    /// referenced record must belong to `order_id`. Calling this again for the
    /// same after-sales record returns the existing fulfillment instead of
    /// writing another row.
    pub async fn create_after_sales_fulfillment(
        &self,
        input: CreateAfterSalesFulfillmentInput,
    ) -> Result<AfterSalesFulfillment, AppError> {
        self.create_after_sales_fulfillment_inner(input, None).await
    }

    /// Persists an after-sales fulfillment and records the workflow audit entry
    /// in the same transaction.
    ///
    /// `consumed_units` is the replacement stock committed by the caller for
    /// this fulfillment and is preserved in the audit payload.
    pub async fn create_after_sales_fulfillment_audited(
        &self,
        input: CreateAfterSalesFulfillmentInput,
        consumed_units: i64,
    ) -> Result<AfterSalesFulfillment, AppError> {
        self.create_after_sales_fulfillment_inner(input, Some(consumed_units))
            .await
    }

    async fn create_after_sales_fulfillment_inner(
        &self,
        input: CreateAfterSalesFulfillmentInput,
        audit_consumed_units: Option<i64>,
    ) -> Result<AfterSalesFulfillment, AppError> {
        match (&input.claim_id, &input.exchange_id) {
            (Some(_), None) | (None, Some(_)) => {}
            _ => {
                return Err(AppError::validation(
                    "an after-sales fulfillment must reference exactly one claim or exchange",
                ));
            }
        }
        if input.items.is_empty() {
            return Err(AppError::validation(
                "an after-sales fulfillment must contain at least one item",
            ));
        }
        for item in &input.items {
            validate_quantity(item.quantity, false)?;
        }
        validate_metadata(&input.metadata)?;
        let provider_id = input.provider_id.trim();
        if provider_id.is_empty() {
            return Err(AppError::validation(
                "provider_id must not be empty".to_owned(),
            ));
        }
        validate_length(provider_id, "provider_id", 64)?;
        let provider_data = if input.provider_data.is_null() {
            serde_json::json!({})
        } else {
            input.provider_data
        };
        if !provider_data.is_object() {
            return Err(AppError::validation(
                "after-sales fulfillment provider_data must be a JSON object",
            ));
        }

        self.get(&input.order_id).await?;
        if let Some(claim_id) = input.claim_id.as_ref() {
            self.load_claim(&input.order_id, claim_id).await?;
            if let Some(existing) = self.claim_fulfillment(&input.order_id, claim_id).await? {
                return Ok(existing);
            }
        }
        if let Some(exchange_id) = input.exchange_id.as_ref() {
            self.load_exchange(&input.order_id, exchange_id).await?;
            if let Some(existing) = self
                .exchange_fulfillment(&input.order_id, exchange_id)
                .await?
            {
                return Ok(existing);
            }
        }

        let id = FulfillmentId::new();
        let now = now_timestamp();
        let mut tx = self.database().begin().await.map_err(AppError::from)?;
        let inserted = sqlx::query(
            "INSERT INTO after_sales_fulfillments
                (id, order_id, claim_id, exchange_id, status, provider_id,
                 provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                 created_at, updated_at)
             VALUES (?, ?, ?, ?, 'fulfilled', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.order_id.as_str())
        .bind(input.claim_id.as_ref().map(ClaimId::as_str))
        .bind(input.exchange_id.as_ref().map(ExchangeId::as_str))
        .bind(provider_id)
        .bind(input.provider_fulfillment_id.as_deref())
        .bind(
            serde_json::to_string(&provider_data)
                .map_err(|error| AppError::validation(format!("invalid provider_data: {error}")))?,
        )
        .bind(
            serde_json::to_string(&input.tracking_numbers).map_err(|error| {
                AppError::validation(format!("invalid tracking numbers: {error}"))
            })?,
        )
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;
        if let Err(error) = inserted {
            if is_unique_violation(&error) {
                // A concurrent runner won the race between the lookup above
                // and this insert. Roll back our partial writes and return the
                // winner's fulfillment.
                tx.rollback().await.map_err(database_error)?;
                let existing = match (&input.claim_id, &input.exchange_id) {
                    (Some(claim_id), None) => {
                        self.claim_fulfillment(&input.order_id, claim_id).await?
                    }
                    (None, Some(exchange_id)) => {
                        self.exchange_fulfillment(&input.order_id, exchange_id)
                            .await?
                    }
                    _ => None,
                };
                return existing.ok_or_else(|| database_error(error));
            }
            return Err(database_error(error));
        }

        for item in &input.items {
            sqlx::query(
                "INSERT INTO after_sales_fulfillment_items
                    (id, fulfillment_id, variant_id, quantity, created_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(EventId::new().as_str())
            .bind(id.as_str())
            .bind(item.variant_id.as_str())
            .bind(item.quantity)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }

        let event_type = if input.claim_id.is_some() {
            "order.claim_fulfilled"
        } else {
            "order.exchange_fulfilled"
        };
        append_after_sales_event(
            &mut tx,
            self.database().store_id(),
            event_type,
            serde_json::json!({
                "order_id": input.order_id.as_str(),
                "fulfillment_id": id.as_str(),
                "claim_id": input.claim_id.as_ref().map(ClaimId::as_str),
                "exchange_id": input.exchange_id.as_ref().map(ExchangeId::as_str),
                "provider_id": provider_id,
            }),
        )
        .await?;

        if let Some(consumed_units) = audit_consumed_units {
            let (action, resource_type, resource_id) = match (&input.claim_id, &input.exchange_id) {
                (Some(claim_id), None) => {
                    ("order.claim_fulfilled", "order_claim", claim_id.as_str())
                }
                (None, Some(exchange_id)) => (
                    "order.exchange_fulfilled",
                    "order_exchange",
                    exchange_id.as_str(),
                ),
                _ => unreachable!("validated above"),
            };
            self.append_audit(
                &mut tx,
                action,
                resource_type,
                resource_id,
                serde_json::json!({
                    "order_id": input.order_id.as_str(),
                    "consumed_units": consumed_units,
                }),
            )
            .await?;
        }

        tx.commit().await.map_err(database_error)?;
        self.load_after_sales_fulfillment(&input.order_id, &id)
            .await
    }

    /// Loads one after-sales fulfillment.
    pub async fn get_after_sales_fulfillment(
        &self,
        order_id: &OrderId,
        fulfillment_id: &FulfillmentId,
    ) -> Result<AfterSalesFulfillment, AppError> {
        self.load_after_sales_fulfillment(order_id, fulfillment_id)
            .await
    }

    /// Returns the single fulfillment recorded for a claim, when present.
    pub async fn claim_fulfillment(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<Option<AfterSalesFulfillment>, AppError> {
        self.find_after_sales_fulfillment(order_id, "claim_id", claim_id.as_str())
            .await
    }

    /// Returns the single fulfillment recorded for an exchange, when present.
    pub async fn exchange_fulfillment(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<Option<AfterSalesFulfillment>, AppError> {
        self.find_after_sales_fulfillment(order_id, "exchange_id", exchange_id.as_str())
            .await
    }

    /// Lists every after-sales fulfillment for an order, oldest first.
    pub async fn list_after_sales_fulfillments(
        &self,
        order_id: &OrderId,
    ) -> Result<Vec<AfterSalesFulfillment>, AppError> {
        self.get(order_id).await?;
        let rows = sqlx::query_as::<_, AfterSalesFulfillmentRow>(
            "SELECT id, order_id, claim_id, exchange_id, status, provider_id,
                    provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                    created_at, updated_at
             FROM after_sales_fulfillments WHERE order_id = ?
             ORDER BY created_at ASC, rowid ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let mut fulfillments = Vec::with_capacity(rows.len());
        for row in rows {
            fulfillments.push(self.after_sales_fulfillment_from_row(row).await?);
        }
        Ok(fulfillments)
    }

    async fn find_after_sales_fulfillment(
        &self,
        order_id: &OrderId,
        column: &str,
        value: &str,
    ) -> Result<Option<AfterSalesFulfillment>, AppError> {
        // `column` is a private constant chosen by the caller, never user input.
        let sql = format!(
            "SELECT id, order_id, claim_id, exchange_id, status, provider_id,
                    provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                    created_at, updated_at
             FROM after_sales_fulfillments
             WHERE order_id = ? AND {column} = ?
             ORDER BY created_at ASC, rowid ASC LIMIT 1"
        );
        let row = sqlx::query_as::<_, AfterSalesFulfillmentRow>(&sql)
            .bind(order_id.as_str())
            .bind(value)
            .fetch_optional(self.database().pool())
            .await
            .map_err(database_error)?;
        match row {
            Some(row) => self.after_sales_fulfillment_from_row(row).await.map(Some),
            None => Ok(None),
        }
    }

    async fn load_after_sales_fulfillment(
        &self,
        order_id: &OrderId,
        fulfillment_id: &FulfillmentId,
    ) -> Result<AfterSalesFulfillment, AppError> {
        let row = sqlx::query_as::<_, AfterSalesFulfillmentRow>(
            "SELECT id, order_id, claim_id, exchange_id, status, provider_id,
                    provider_fulfillment_id, provider_data, tracking_numbers, metadata,
                    created_at, updated_at
             FROM after_sales_fulfillments WHERE id = ? AND order_id = ?",
        )
        .bind(fulfillment_id.as_str())
        .bind(order_id.as_str())
        .fetch_optional(self.database().pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("after-sales fulfillment", fulfillment_id))?;
        self.after_sales_fulfillment_from_row(row).await
    }

    async fn after_sales_fulfillment_from_row(
        &self,
        row: AfterSalesFulfillmentRow,
    ) -> Result<AfterSalesFulfillment, AppError> {
        let item_rows = sqlx::query_as::<_, AfterSalesFulfillmentItemRow>(
            "SELECT id, fulfillment_id, variant_id, quantity, created_at
             FROM after_sales_fulfillment_items WHERE fulfillment_id = ?
             ORDER BY rowid ASC",
        )
        .bind(&row.id)
        .fetch_all(self.database().pool())
        .await
        .map_err(database_error)?;
        let items = item_rows
            .into_iter()
            .map(AfterSalesFulfillmentItemRow::into_domain)
            .collect();
        Ok(AfterSalesFulfillment {
            id: FulfillmentId::from_string(row.id),
            order_id: OrderId::from_string(row.order_id),
            claim_id: row.claim_id.map(ClaimId::from_string),
            exchange_id: row.exchange_id.map(ExchangeId::from_string),
            status: row.status,
            provider_id: row.provider_id,
            provider_fulfillment_id: row.provider_fulfillment_id,
            provider_data: serde_json::from_str(&row.provider_data)
                .unwrap_or(serde_json::Value::Null),
            tracking_numbers: serde_json::from_str(&row.tracking_numbers).unwrap_or_default(),
            metadata: fastcart_kernel::Metadata::from_json_str(&row.metadata),
            created_at: row.created_at,
            updated_at: row.updated_at,
            items,
        })
    }
}
fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
}
