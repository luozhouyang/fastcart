//! Gift card issuing, management and redemption.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CurrencyCode, PageResult, Pagination, now_timestamp, validate_metadata,
};
use rand::RngCore;
use sqlx::{Sqlite, Transaction};

use crate::domain::{CreateGiftCardInput, GiftCard, GiftCardStatus};

use super::store::*;
use super::*;

/// Columns selected for every gift card read.
const GIFT_CARD_COLUMNS: &str = "id, code, currency_code, initial_amount_minor, balance_minor, \
                                 status, metadata, created_at, updated_at";

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
        let updated = sqlx::query(
            "UPDATE gift_cards SET balance_minor = balance_minor - ?, updated_at = ? \
             WHERE id = ? AND balance_minor >= ?",
        )
        .bind(amount_minor)
        .bind(now_timestamp())
        .bind(&card.id)
        .bind(amount_minor)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "gift card {} has insufficient balance: {} remaining, {amount_minor} requested",
                card.id, card.balance_minor
            )));
        }
        let redeemed = load_gift_card(&mut tx, &card.id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(redeemed.balance_minor)
    }
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
