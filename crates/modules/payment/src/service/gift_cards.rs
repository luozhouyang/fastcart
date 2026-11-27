//! Gift card issuing, management and redemption.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use std::collections::BTreeMap;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CheckoutId, CurrencyCode, OrderId, PageResult, Pagination, now_timestamp,
    validate_metadata,
};
use rand::RngCore;
use sqlx::{FromRow, Sqlite, Transaction};

use crate::domain::{
    CreateGiftCardInput, GiftCard, GiftCardRedemption, GiftCardRedemptionStatus,
    GiftCardRefundCredit, GiftCardStatus,
};

use super::store::*;
use super::*;

/// Columns selected for every gift card read.
const GIFT_CARD_COLUMNS: &str = "id, code, currency_code, initial_amount_minor, balance_minor, \
                                 status, metadata, created_at, updated_at";

/// Columns selected for every gift card redemption read.
const GIFT_CARD_REDEMPTION_COLUMNS: &str = "id, gift_card_id, checkout_id, order_id, \
     amount_minor, status, created_at, updated_at, reversed_at";

/// Columns selected for every gift card refund credit read.
const GIFT_CARD_REFUND_CREDIT_COLUMNS: &str =
    "id, refund_id, redemption_id, gift_card_id, amount_minor, created_at";

/// One `payment_gift_card_redemptions` row.
#[derive(Debug, FromRow)]
struct GiftCardRedemptionRow {
    id: String,
    gift_card_id: String,
    checkout_id: String,
    order_id: Option<String>,
    amount_minor: i64,
    status: String,
    created_at: i64,
    updated_at: i64,
    reversed_at: Option<i64>,
}

/// Converts a stored redemption row.
///
/// An unknown status fails loudly instead of guessing: the ledger decides
/// whether a completion may spend a card again, so a value written by a newer
/// build must not be read as either.
fn gift_card_redemption_from_row(
    row: GiftCardRedemptionRow,
) -> Result<GiftCardRedemption, AppError> {
    let status = GiftCardRedemptionStatus::parse(&row.status).ok_or_else(|| {
        AppError::database(format!(
            "unknown gift card redemption status {} on {}",
            row.status, row.id
        ))
    })?;
    Ok(GiftCardRedemption {
        id: row.id,
        gift_card_id: row.gift_card_id,
        checkout_id: row.checkout_id,
        order_id: row.order_id,
        amount_minor: row.amount_minor,
        status,
        created_at: row.created_at,
        updated_at: row.updated_at,
        reversed_at: row.reversed_at,
    })
}

/// One `gift_card_refund_credits` row.
#[derive(Debug, FromRow)]
struct GiftCardRefundCreditRow {
    id: String,
    refund_id: String,
    redemption_id: String,
    gift_card_id: String,
    amount_minor: i64,
    created_at: i64,
}

/// Converts a stored refund credit row.
fn gift_card_refund_credit_from_row(row: GiftCardRefundCreditRow) -> GiftCardRefundCredit {
    GiftCardRefundCredit {
        id: row.id,
        refund_id: row.refund_id,
        redemption_id: row.redemption_id,
        gift_card_id: row.gift_card_id,
        amount_minor: row.amount_minor,
        created_at: row.created_at,
    }
}

/// One still-redeemed order line and how much earlier refunds already credited.
#[derive(Debug, FromRow)]
struct RefundableRedemptionRow {
    redemption_id: String,
    gift_card_id: String,
    amount_minor: i64,
    credited_minor: i64,
}

/// Alphabet for generated redemption codes.
///
/// Crockford-style base32 without `I`, `L`, `O` and `U`: the characters that
/// look like `1`, `0`, `4` or another letter when a code is read over the
/// phone or written down. Case-insensitive lookups are still not offered —
/// codes are matched exactly as issued.
const CODE_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Number of characters in a generated redemption code.
const CODE_LENGTH: usize = 16;

/// Generates an opaque redemption code.
///
/// The code is a bearer credential worth its balance, so it is drawn from the
/// operating system's cryptographic random source rather than a seeded RNG.
fn generate_gift_card_code() -> String {
    let mut bytes = [0_u8; CODE_LENGTH];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    // 32 alphabet entries divide 256 exactly, so masking with 31 keeps the
    // distribution uniform without rejection sampling.
    bytes
        .iter()
        .map(|byte| CODE_ALPHABET[usize::from(byte & 0x1f)] as char)
        .collect()
}

/// Loads a gift card by identifier inside an open transaction.
async fn load_gift_card(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<GiftCard, AppError> {
    let sql = format!("SELECT {GIFT_CARD_COLUMNS} FROM gift_cards WHERE id = ?");
    let row = sqlx::query_as::<_, GiftCardRow>(&sql)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;
    row.map(gift_card_from_row)
        .ok_or_else(|| AppError::not_found("gift card", id))
}

impl PaymentService {
    /// Issues a gift card holding `input.amount_minor`.
    ///
    /// The amount must be positive and the currency a valid ISO-4217 code; the
    /// generated code is unique by construction, while a caller-supplied code
    /// collides with an existing one as a conflict.
    pub async fn issue_gift_card(&self, input: CreateGiftCardInput) -> Result<GiftCard, AppError> {
        if input.amount_minor <= 0 {
            return Err(AppError::validation("gift card amount must be positive"));
        }
        validate_metadata(&input.metadata)?;
        let currency = CurrencyCode::new(input.currency_code)?;
        let code = match input.code.map(|code| code.trim().to_owned()) {
            Some(code) if !code.is_empty() => code,
            Some(_) => {
                return Err(AppError::validation(
                    "gift card code must not be empty when supplied",
                ));
            }
            None => generate_gift_card_code(),
        };
        if code.chars().count() > 64 {
            return Err(AppError::validation(
                "gift card code must not exceed 64 characters",
            ));
        }

        let id = format!("gift_{}", uuid::Uuid::now_v7().simple());
        let now = now_timestamp();
        // `RETURNING` keeps the response consistent with the committed row: a
        // follow-up read could observe the card missing after a concurrent
        // delete while its code is already spendable.
        let sql = format!(
            "INSERT INTO gift_cards \
             (id, code, currency_code, initial_amount_minor, balance_minor, status, \
              metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 'active', ?, ?, ?) \
             RETURNING {GIFT_CARD_COLUMNS}"
        );
        let row = sqlx::query_as::<_, GiftCardRow>(&sql)
            .bind(&id)
            .bind(&code)
            .bind(currency.as_str())
            .bind(input.amount_minor)
            .bind(input.amount_minor)
            .bind(input.metadata.to_json_string())
            .bind(now)
            .bind(now)
            .fetch_one(self.db.pool())
            .await;
        match row {
            Ok(row) => Ok(gift_card_from_row(row)),
            Err(error) => Err(conflict_on_unique(
                error,
                format!("gift card code {code} already exists"),
            )),
        }
    }

    /// Loads a gift card.
    pub async fn get_gift_card(&self, id: &str) -> Result<GiftCard, AppError> {
        let sql = format!("SELECT {GIFT_CARD_COLUMNS} FROM gift_cards WHERE id = ?");
        let row = sqlx::query_as::<_, GiftCardRow>(&sql)
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        row.map(gift_card_from_row)
            .ok_or_else(|| AppError::not_found("gift card", id))
    }

    /// Loads a gift card by the code a customer presents.
    ///
    /// A caller that only holds the code (for example a checkout surface
    /// validating a redemption) uses this to check the currency before
    /// calling [`Self::redeem_gift_card`].
    pub async fn find_gift_card_by_code(&self, code: &str) -> Result<GiftCard, AppError> {
        let sql = format!("SELECT {GIFT_CARD_COLUMNS} FROM gift_cards WHERE code = ?");
        let row = sqlx::query_as::<_, GiftCardRow>(&sql)
            .bind(code)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        row.map(gift_card_from_row)
            .ok_or_else(|| AppError::not_found("gift card", code))
    }

    /// Lists gift cards, newest first, with offset/limit pagination.
    pub async fn list_gift_cards(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<GiftCard>, AppError> {
        let sql = format!(
            "SELECT {GIFT_CARD_COLUMNS} FROM gift_cards \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query_as::<_, GiftCardRow>(&sql)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gift_cards")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(PageResult::new(
            rows.into_iter().map(gift_card_from_row).collect(),
            count,
            pagination,
        ))
    }

    /// Disables a gift card, rejecting all further redemptions.
    ///
    /// Disabling an already disabled card is a no-op, so the call is
    /// idempotent.
    pub async fn disable_gift_card(&self, id: &str) -> Result<GiftCard, AppError> {
        self.set_gift_card_status(id, GiftCardStatus::Disabled)
            .await
    }

    /// Enables a disabled gift card again.
    ///
    /// Enabling an already active card is a no-op.
    pub async fn enable_gift_card(&self, id: &str) -> Result<GiftCard, AppError> {
        self.set_gift_card_status(id, GiftCardStatus::Active).await
    }

    async fn set_gift_card_status(
        &self,
        id: &str,
        status: GiftCardStatus,
    ) -> Result<GiftCard, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let current = load_gift_card(&mut tx, id).await?;
        if current.status == status {
            // Already in the requested state: leave `updated_at` alone so the
            // call is a true no-op.
            return Ok(current);
        }
        sqlx::query("UPDATE gift_cards SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(now_timestamp())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        let card = load_gift_card(&mut tx, id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(card)
    }

    /// Redeems `amount_minor` from the card behind `code`, returning the new
    /// balance in minor units.
    ///
    /// The caller checks the currency of the purchase against
    /// [`GiftCard::currency_code`]; the service only enforces the balance and
    /// the card status. The decrement is a conditional update guarded by
    /// `balance_minor >= amount`, executed inside a `BEGIN IMMEDIATE`
    /// transaction and checked through `rows_affected`, so two concurrent
    /// redemptions at the boundary cannot both commit: the loser observes no
    /// affected row and fails with a conflict instead of overselling.
    pub async fn redeem_gift_card(&self, code: &str, amount_minor: i64) -> Result<i64, AppError> {
        if amount_minor <= 0 {
            return Err(AppError::validation(
                "gift card redemption amount must be positive",
            ));
        }
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let row = sqlx::query_as::<_, GiftCardRow>(
            "SELECT id, code, currency_code, initial_amount_minor, balance_minor, status, \
             metadata, created_at, updated_at FROM gift_cards WHERE code = ?",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let card = row
            .map(gift_card_from_row)
            .ok_or_else(|| AppError::not_found("gift card", code))?;
        if card.status != GiftCardStatus::Active {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "gift card {} is not active",
                card.id
            )));
        }
        if let Err(error) = decrement_gift_card_balance(&mut tx, &card, amount_minor).await {
            tx.rollback().await.map_err(database_error)?;
            return Err(error);
        }
        let redeemed = load_gift_card(&mut tx, &card.id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(redeemed.balance_minor)
    }

    /// Redeems one gift card for a checkout and records the redemption.
    ///
    /// The guarded balance decrement and the ledger row are written in the same
    /// `BEGIN IMMEDIATE` transaction, so a completion that crashes after
    /// spending a card always leaves the record behind. The ledger is unique per
    /// `(checkout_id, gift_card_id)`: a retried completion finds the recorded
    /// redemption, verifies the amount still matches the quote and returns it
    /// instead of spending the card twice. An earlier redemption that a
    /// compensation reversed is re-applied with the same guarded decrement.
    ///
    /// This is the checkout entry point into the same guarded update that
    /// [`Self::redeem_gift_card`] uses. The card's currency is the caller's
    /// responsibility, checked when the card was applied to the cart.
    pub async fn redeem_gift_card_for_checkout(
        &self,
        checkout_id: &CheckoutId,
        gift_card_id: &str,
        amount_minor: i64,
    ) -> Result<GiftCardRedemption, AppError> {
        if amount_minor <= 0 {
            return Err(AppError::validation(
                "gift card redemption amount must be positive",
            ));
        }
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        if let Some(existing) =
            load_redemption_locked(&mut tx, checkout_id.as_str(), gift_card_id).await?
        {
            match existing.status {
                GiftCardRedemptionStatus::Redeemed => {
                    if existing.amount_minor != amount_minor {
                        tx.rollback().await.map_err(database_error)?;
                        return Err(AppError::conflict(format!(
                            "gift card redemption for checkout {checkout_id} is already recorded \
                             for {}, not {amount_minor}",
                            existing.amount_minor
                        )));
                    }
                    tx.rollback().await.map_err(database_error)?;
                    return Ok(existing);
                }
                GiftCardRedemptionStatus::Reversed => {}
            }
        }
        let card = load_gift_card(&mut tx, gift_card_id).await?;
        if card.status != GiftCardStatus::Active {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "gift card {} is not active",
                card.id
            )));
        }
        if let Err(error) = decrement_gift_card_balance(&mut tx, &card, amount_minor).await {
            tx.rollback().await.map_err(database_error)?;
            return Err(error);
        }
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO payment_gift_card_redemptions \
             (id, gift_card_id, checkout_id, order_id, amount_minor, status, created_at, \
              updated_at, reversed_at) \
             VALUES (?, ?, ?, NULL, ?, 'redeemed', ?, ?, NULL) \
             ON CONFLICT (checkout_id, gift_card_id) DO UPDATE SET \
              amount_minor = excluded.amount_minor, status = 'redeemed', \
              updated_at = excluded.updated_at, reversed_at = NULL",
        )
        .bind(format!("gcred_{}", uuid::Uuid::now_v7().simple()))
        .bind(&card.id)
        .bind(checkout_id.as_str())
        .bind(amount_minor)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        let redemption = load_redemption_locked(&mut tx, checkout_id.as_str(), &card.id)
            .await?
            .ok_or_else(|| AppError::internal("gift card redemption vanished after insert"))?;
        tx.commit().await.map_err(database_error)?;
        Ok(redemption)
    }

    /// Lists the redemptions a checkout recorded, newest first.
    pub async fn gift_card_redemptions_for_checkout(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<Vec<GiftCardRedemption>, AppError> {
        let rows = sqlx::query_as::<_, GiftCardRedemptionRow>(&format!(
            "SELECT {GIFT_CARD_REDEMPTION_COLUMNS} FROM payment_gift_card_redemptions \
             WHERE checkout_id = ? ORDER BY created_at ASC, id ASC"
        ))
        .bind(checkout_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter()
            .map(gift_card_redemption_from_row)
            .collect()
    }

    /// Lists the checkout and order a gift card paid for, newest first.
    ///
    /// This is the admin view of a card's history: the order identifier is
    /// filled in once the order is placed, so an operator can reconcile a card
    /// against the orders it paid for.
    pub async fn gift_card_redemptions_for_card(
        &self,
        gift_card_id: &str,
    ) -> Result<Vec<GiftCardRedemption>, AppError> {
        let rows = sqlx::query_as::<_, GiftCardRedemptionRow>(&format!(
            "SELECT {GIFT_CARD_REDEMPTION_COLUMNS} FROM payment_gift_card_redemptions \
             WHERE gift_card_id = ? ORDER BY created_at DESC, id DESC"
        ))
        .bind(gift_card_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter()
            .map(gift_card_redemption_from_row)
            .collect()
    }

    /// Lists the redemptions a placed order paid with, oldest first.
    ///
    /// Refund allocation walks this order: the oldest redemption is credited
    /// back before a newer one.
    pub async fn gift_card_redemptions_for_order(
        &self,
        order_id: &OrderId,
    ) -> Result<Vec<GiftCardRedemption>, AppError> {
        let rows = sqlx::query_as::<_, GiftCardRedemptionRow>(&format!(
            "SELECT {GIFT_CARD_REDEMPTION_COLUMNS} FROM payment_gift_card_redemptions \
             WHERE order_id = ? ORDER BY created_at ASC, id ASC"
        ))
        .bind(order_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter()
            .map(gift_card_redemption_from_row)
            .collect()
    }

    /// Lists the gift card credits one refund recorded, oldest first.
    pub async fn gift_card_refund_credits_for_refund(
        &self,
        refund_id: &str,
    ) -> Result<Vec<GiftCardRefundCredit>, AppError> {
        let rows = sqlx::query_as::<_, GiftCardRefundCreditRow>(&format!(
            "SELECT {GIFT_CARD_REFUND_CREDIT_COLUMNS} FROM gift_card_refund_credits \
             WHERE refund_id = ? ORDER BY created_at ASC, id ASC"
        ))
        .bind(refund_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(gift_card_refund_credit_from_row)
            .collect())
    }

    /// Amount one refund credited back to gift cards, in minor units.
    pub async fn gift_card_refunded_for_refund(&self, refund_id: &str) -> Result<i64, AppError> {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_minor), 0) FROM gift_card_refund_credits \
             WHERE refund_id = ?",
        )
        .bind(refund_id)
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)
    }

    /// Amount each refund of a payment credited back to gift cards.
    ///
    /// Refunds that credited nothing are absent from the map; callers read a
    /// missing identifier as zero. One grouped query serves a whole refund
    /// list without a query per row.
    pub async fn gift_card_refund_totals_for_payment(
        &self,
        payment_id: &str,
    ) -> Result<BTreeMap<String, i64>, AppError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT c.refund_id, SUM(c.amount_minor) FROM gift_card_refund_credits c \
             JOIN refunds r ON r.id = c.refund_id \
             WHERE r.payment_id = ? GROUP BY c.refund_id",
        )
        .bind(payment_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows.into_iter().collect())
    }

    /// Names the order the checkout's redemptions paid for.
    ///
    /// Idempotent: replaying it after a crash rewrites the same identifier.
    pub async fn assign_gift_card_redemptions_to_order(
        &self,
        checkout_id: &CheckoutId,
        order_id: &OrderId,
    ) -> Result<u64, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let updated = sqlx::query(
            "UPDATE payment_gift_card_redemptions SET order_id = ?, updated_at = ? \
             WHERE checkout_id = ? AND (order_id IS NULL OR order_id != ?)",
        )
        .bind(order_id.as_str())
        .bind(now)
        .bind(checkout_id.as_str())
        .bind(order_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(updated.rows_affected())
    }

    /// Credits the checkout's still-redeemed cards back and marks them reversed.
    ///
    /// Called when a completion is compensated: the order the cards paid for
    /// will never exist, so the money must return to them. Idempotent through
    /// the `status = 'redeemed'` guard, and the balance credit and the status
    /// flip share one `BEGIN IMMEDIATE` transaction. Returns the number of
    /// redemptions reversed.
    pub async fn reverse_gift_card_redemptions(
        &self,
        checkout_id: &CheckoutId,
    ) -> Result<u64, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let rows = sqlx::query_as::<_, GiftCardRedemptionRow>(&format!(
            "SELECT {GIFT_CARD_REDEMPTION_COLUMNS} FROM payment_gift_card_redemptions \
             WHERE checkout_id = ? AND status = 'redeemed' ORDER BY created_at ASC, id ASC"
        ))
        .bind(checkout_id.as_str())
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?;
        let mut reversed = 0_u64;
        for row in rows {
            let flipped = sqlx::query(
                "UPDATE payment_gift_card_redemptions SET status = 'reversed', reversed_at = ?, \
                 updated_at = ? WHERE id = ? AND status = 'redeemed'",
            )
            .bind(now)
            .bind(now)
            .bind(&row.id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            if flipped.rows_affected() == 0 {
                continue;
            }
            sqlx::query(
                "UPDATE gift_cards SET balance_minor = balance_minor + ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(row.amount_minor)
            .bind(now)
            .bind(&row.gift_card_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            reversed += 1;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(reversed)
    }

    /// Credits up to `amount_minor` of `refund_id` back to the order's cards.
    ///
    /// The caller runs this inside the refund claim's `BEGIN IMMEDIATE`
    /// transaction, before the provider is asked for any remainder. Running the
    /// allocation there is what keeps concurrent refunds honest: the write lock
    /// serializes the claims, and each claim sums every credit an earlier
    /// refund already wrote, so two refunds can never both credit the same
    /// part of one redemption.
    ///
    /// Allocation walks the order's still-redeemed lines oldest first and
    /// credits at most the part of each line that no refund has credited yet.
    /// The credit ledger row, the balance increase and the refund row all
    /// commit together; a crash before the provider call leaves the cards
    /// credited and the refund pending, which a retry with the same key
    /// reconciles instead of crediting twice. Returns the amount credited.
    pub(super) async fn allocate_refund_to_gift_cards(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        refund_id: &str,
        order_id: Option<&OrderId>,
        amount_minor: i64,
    ) -> Result<i64, AppError> {
        let Some(order_id) = order_id else {
            return Ok(0);
        };
        if amount_minor <= 0 {
            return Ok(0);
        }
        let rows = sqlx::query_as::<_, RefundableRedemptionRow>(
            "SELECT r.id AS redemption_id, r.gift_card_id, r.amount_minor, \
                    COALESCE(SUM(c.amount_minor), 0) AS credited_minor \
             FROM payment_gift_card_redemptions r \
             LEFT JOIN gift_card_refund_credits c ON c.redemption_id = r.id \
             WHERE r.order_id = ? AND r.status = 'redeemed' \
             GROUP BY r.id, r.gift_card_id, r.amount_minor, r.created_at \
             ORDER BY r.created_at ASC, r.id ASC",
        )
        .bind(order_id.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(database_error)?;
        let now = now_timestamp();
        let mut remaining = amount_minor;
        for row in rows {
            if remaining == 0 {
                break;
            }
            let available = row.amount_minor.saturating_sub(row.credited_minor);
            if available <= 0 {
                continue;
            }
            let credited = available.min(remaining);
            sqlx::query(
                "INSERT INTO gift_card_refund_credits \
                 (id, refund_id, redemption_id, gift_card_id, amount_minor, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("gcredit_{}", uuid::Uuid::now_v7().simple()))
            .bind(refund_id)
            .bind(&row.redemption_id)
            .bind(&row.gift_card_id)
            .bind(credited)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
            // Same guarded shape as the redemption decrement: the update is
            // conditional on the card still existing and the affected-row
            // count decides whether the ledger row may stand.
            let updated = sqlx::query(
                "UPDATE gift_cards SET balance_minor = balance_minor + ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(credited)
            .bind(now)
            .bind(&row.gift_card_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                return Err(AppError::internal(format!(
                    "gift card {} vanished while refund {refund_id} credited it",
                    row.gift_card_id
                )));
            }
            remaining -= credited;
        }
        Ok(amount_minor - remaining)
    }

    /// Removes every gift card credit a refund recorded, debiting the money.
    ///
    /// A retried refund recomputes its allocation from scratch, so the credits
    /// of its earlier attempt are taken back first; a definitively rejected
    /// refund is rolled back the same way, keeping cards and ledger in step.
    ///
    /// The debit is unconditional on purpose: the money was added by this
    /// refund, and leaving it on a card after its ledger row disappears would
    /// create money. Only a card whose balance was spent again after the
    /// credit can go negative; it then rejects further redemptions until the
    /// merchant reconciles it. The caller runs this in the same transaction
    /// that changes the refund row.
    pub(super) async fn reverse_refund_gift_card_credits(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        refund_id: &str,
    ) -> Result<i64, AppError> {
        let rows = sqlx::query_as::<_, GiftCardRefundCreditRow>(&format!(
            "SELECT {GIFT_CARD_REFUND_CREDIT_COLUMNS} FROM gift_card_refund_credits \
             WHERE refund_id = ? ORDER BY created_at ASC, id ASC"
        ))
        .bind(refund_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(database_error)?;
        if rows.is_empty() {
            return Ok(0);
        }
        let now = now_timestamp();
        let mut reversed = 0_i64;
        for row in &rows {
            let updated = sqlx::query(
                "UPDATE gift_cards SET balance_minor = balance_minor - ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(row.amount_minor)
            .bind(now)
            .bind(&row.gift_card_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                return Err(AppError::internal(format!(
                    "gift card {} vanished while refund {refund_id} reversed its credit",
                    row.gift_card_id
                )));
            }
            reversed += row.amount_minor;
        }
        sqlx::query("DELETE FROM gift_card_refund_credits WHERE refund_id = ?")
            .bind(refund_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
        Ok(reversed)
    }
}

/// Applies the guarded balance decrement inside an open transaction.
///
/// The `balance_minor >= ?` predicate and the `rows_affected` check are the
/// single place gift card money leaves a card: two concurrent callers that
/// together ask for more than the balance cannot both commit, because the
/// second update matches no row and fails with a conflict.
async fn decrement_gift_card_balance(
    tx: &mut Transaction<'_, Sqlite>,
    card: &GiftCard,
    amount_minor: i64,
) -> Result<(), AppError> {
    let updated = sqlx::query(
        "UPDATE gift_cards SET balance_minor = balance_minor - ?, updated_at = ? \
         WHERE id = ? AND balance_minor >= ?",
    )
    .bind(amount_minor)
    .bind(now_timestamp())
    .bind(&card.id)
    .bind(amount_minor)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    if updated.rows_affected() == 0 {
        return Err(AppError::conflict(format!(
            "gift card {} has insufficient balance: {} remaining, {amount_minor} requested",
            card.id, card.balance_minor
        )));
    }
    Ok(())
}

/// Loads the redemption of one card by one checkout, if it exists.
async fn load_redemption_locked(
    tx: &mut Transaction<'_, Sqlite>,
    checkout_id: &str,
    gift_card_id: &str,
) -> Result<Option<GiftCardRedemption>, AppError> {
    let row = sqlx::query_as::<_, GiftCardRedemptionRow>(&format!(
        "SELECT {GIFT_CARD_REDEMPTION_COLUMNS} FROM payment_gift_card_redemptions \
         WHERE checkout_id = ? AND gift_card_id = ?"
    ))
    .bind(checkout_id)
    .bind(gift_card_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    row.map(gift_card_redemption_from_row).transpose()
}

/// Maps a SQLite unique violation to a conflict, leaving other errors alone.
fn conflict_on_unique(error: sqlx::Error, message: String) -> AppError {
    if let sqlx::Error::Database(database) = &error
        && (database.is_unique_violation()
            || database
                .message()
                .to_ascii_lowercase()
                .contains("unique constraint"))
    {
        return AppError::conflict(message);
    }
    database_error(error)
}
