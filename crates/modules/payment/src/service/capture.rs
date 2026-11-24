//! Captures and the capture ledger.

//! SQLite-backed payment application service.
//!
//! Provider calls always happen outside database transactions. Multi-row state
//! changes (session + attempt, payment + refund) use one short transaction so a
//! failed write cannot leave a payment half-applied.

use std::collections::BTreeMap;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, Metadata, PaymentAttemptId, PaymentId, now_timestamp};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::{Payment, PaymentAttempt, PaymentSession, PaymentStatus};
use crate::provider::{CaptureRequest, CaptureResult, PaymentError};

use super::store::*;
use super::*;
/// Ledger field of the capture bookkeeping inside the session data.
const CAPTURE_LEDGER_FIELD: &str = "capture";
/// Claim field holding the provider idempotency key of the in-flight capture.
const CAPTURE_CLAIM_FIELD: &str = "capture_claim";
/// Timestamp of the in-flight capture claim, used to expire stale claims.
const CAPTURE_CLAIM_AT_FIELD: &str = "capture_claim_at";

/// One claimed capture: the provider idempotency key (which is also the claim
/// token), the ledger key that identifies the logical capture and the reserved
/// amount.
struct CaptureClaim {
    token: String,
    ledger_key: String,
    amount: i64,
}

/// Outcome of trying to claim a capture.
enum CaptureClaimOutcome {
    /// The capture is claimed and its amount reserved; the provider may be
    /// called with the claim token as the idempotency key.
    Claimed(CaptureClaim),
    /// The session already recorded this capture; the recorded payment is
    /// returned and the provider is not called again.
    AlreadyCaptured(Payment),
}

/// State of one logical capture in the session capture ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureEntryState {
    /// The provider call is outstanding or its outcome is unknown; the amount
    /// is reserved and counted against the cumulative cap.
    InFlight,
    /// The provider confirmed the capture; the amount is part of the payment.
    Settled,
    /// The provider definitively rejected the capture; the amount is neither
    /// captured nor reserved any more.
    Failed,
}

impl CaptureEntryState {
    fn as_str(self) -> &'static str {
        match self {
            Self::InFlight => "in_flight",
            Self::Settled => "settled",
            Self::Failed => "failed",
        }
    }

    /// Parses a stored state. Unknown values count as in flight so a record
    /// written by a newer build can only under-state the remaining amount, not
    /// over-state it.
    fn parse(value: &str) -> Self {
        match value {
            "settled" => Self::Settled,
            "failed" => Self::Failed,
            _ => Self::InFlight,
        }
    }
}

/// One capture reservation in the session ledger.
#[derive(Clone, Copy, Debug)]
struct CaptureEntry {
    amount: i64,
    state: CaptureEntryState,
    at: i64,
}

/// Durable ledger of the captures of one payment session, stored as
/// `data.capture` because one session may be captured in several parts.
#[derive(Clone, Debug, Default)]
struct CaptureLedger {
    entries: BTreeMap<String, CaptureEntry>,
}

impl CaptureLedger {
    fn from_metadata(data: &Metadata) -> Self {
        let Some(entries) = data
            .get(CAPTURE_LEDGER_FIELD)
            .and_then(|ledger| ledger.get("entries"))
            .and_then(Value::as_object)
        else {
            return Self::default();
        };
        let mut ledger = Self::default();
        for (key, entry) in entries {
            let Some(amount) = entry.get("amount").and_then(Value::as_i64) else {
                continue;
            };
            let state = entry
                .get("state")
                .and_then(Value::as_str)
                .map_or(CaptureEntryState::InFlight, CaptureEntryState::parse);
            let at = entry.get("at").and_then(Value::as_i64).unwrap_or_default();
            ledger.insert(key, CaptureEntry { amount, state, at });
        }
        ledger
    }

    fn to_value(&self) -> Value {
        let entries = self
            .entries
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    serde_json::json!({
                        "amount": entry.amount,
                        "state": entry.state.as_str(),
                        "at": entry.at,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        Value::Object(serde_json::Map::from_iter([(
            "entries".to_owned(),
            Value::Object(entries),
        )]))
    }

    fn insert(&mut self, key: &str, entry: CaptureEntry) {
        self.entries.insert(key.to_owned(), entry);
    }

    fn get(&self, key: &str) -> Option<&CaptureEntry> {
        self.entries.get(key)
    }

    /// Amount the provider confirmed for this session.
    fn settled_total(&self) -> i64 {
        self.entries
            .values()
            .filter(|entry| entry.state == CaptureEntryState::Settled)
            .map(|entry| entry.amount)
            .sum()
    }

    /// Amount the provider may already have captured: everything except
    /// definitively rejected captures.
    fn reserved_total(&self) -> i64 {
        self.entries
            .values()
            .filter(|entry| entry.state != CaptureEntryState::Failed)
            .map(|entry| entry.amount)
            .sum()
    }
}

/// Returns true when the session already records a captured payment.
fn capture_settled(session: &PaymentSession) -> bool {
    matches!(
        session.status,
        PaymentStatus::Captured | PaymentStatus::PartiallyRefunded | PaymentStatus::Refunded
    )
}

/// Trims a caller idempotency key; blank keys behave like no key.
fn normalize_idempotency_key(idempotency_key: Option<String>) -> Option<String> {
    idempotency_key
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
}

/// Derives the ledger key of a keyless capture.
///
/// The key is stable across retries of the same logical capture because it only
/// depends on the session, the requested amount and the amount captured before
/// the call. The first full capture of a session keeps the historical empty key
/// so its provider idempotency key stays `capture:{session_id}` and a retry
/// started by an older build still deduplicates at the provider.
fn keyless_capture_key(
    session_id: &str,
    settled_before: i64,
    amount: i64,
    session_amount: i64,
) -> String {
    if settled_before == 0 && amount == session_amount {
        return String::new();
    }
    let mut hasher = Sha256::new();
    hasher.update(b"fastcart.capture.v1\0");
    hasher.update((session_id.len() as u64).to_be_bytes());
    hasher.update(session_id.as_bytes());
    hasher.update(settled_before.to_be_bytes());
    hasher.update(amount.to_be_bytes());
    format!("capture:auto:{}", hex::encode(hasher.finalize()))
}

/// Provider idempotency key (and claim token) of one logical capture.
fn capture_provider_key(session_id: &str, ledger_key: &str) -> String {
    if ledger_key.is_empty() {
        format!("capture:{session_id}")
    } else {
        format!("capture:{session_id}:{ledger_key}")
    }
}

/// Session data copied onto a payment, without the internal capture keys.
fn payment_metadata(data: &Metadata) -> Metadata {
    let mut metadata = data.clone();
    metadata.remove(CAPTURE_LEDGER_FIELD);
    metadata.remove(CAPTURE_CLAIM_FIELD);
    metadata.remove(CAPTURE_CLAIM_AT_FIELD);
    metadata
}

impl PaymentService {
    /// Captures a payment session, returning the existing payment when the
    /// session was already captured.
    ///
    /// This entry point keeps its original meaning: it captures the whole
    /// remaining session amount, so a session that was partially captured is
    /// completed instead of charged twice.
    pub async fn capture(&self, session_id: &str) -> Result<Payment, AppError> {
        self.capture_internal(session_id, None, None).await
    }

    /// Captures `amount` of a payment session.
    ///
    /// An amount below the remaining session amount is a partial capture: later
    /// calls may capture the rest, and the sum of every capture never exceeds
    /// the session amount. The provider idempotency key is derived
    /// deterministically from the session and the amounts involved, so an
    /// identical retry resumes the same capture instead of charging again.
    pub async fn capture_amount(&self, session_id: &str, amount: i64) -> Result<Payment, AppError> {
        self.capture_amount_with_idempotency(session_id, amount, None)
            .await
    }

    /// Captures `amount` of a payment session with an optional caller
    /// idempotency key.
    ///
    /// A repeated key returns the payment the first call recorded without
    /// calling the provider again. Reusing a key for a different amount is a
    /// conflict: one key identifies one logical capture.
    pub async fn capture_amount_with_idempotency(
        &self,
        session_id: &str,
        amount: i64,
        idempotency_key: Option<String>,
    ) -> Result<Payment, AppError> {
        if amount <= 0 {
            return Err(AppError::validation("capture amount must be positive"));
        }
        self.capture_internal(session_id, Some(amount), idempotency_key)
            .await
    }

    /// Captures part or all of the remaining session amount.
    ///
    /// `requested` is `None` for the whole remaining amount. The amount cap and
    /// the idempotency lookup share one `BEGIN IMMEDIATE` transaction with the
    /// claim, so concurrent callers can never capture more than the session
    /// amount in total:
    ///
    /// * the claim (`$.capture_claim`) admits one in-flight capture at a time,
    /// * every accepted capture reserves its amount in `$.capture.entries`, so
    ///   any further capture is capped by the sum of the reservations,
    /// * an ambiguous provider/transport failure keeps its reservation counted
    ///   (like a pending refund) and releases the claim, and a retry resumes the
    ///   same entry with the same provider idempotency key,
    /// * a definitive rejection releases the reservation.
    async fn capture_internal(
        &self,
        session_id: &str,
        requested: Option<i64>,
        idempotency_key: Option<String>,
    ) -> Result<Payment, AppError> {
        let session = self.get_session(session_id).await?;
        if capture_settled(&session) {
            return self.payment_for_session(&session).await;
        }
        if matches!(
            session.status,
            PaymentStatus::Failed | PaymentStatus::Canceled
        ) {
            return Err(AppError::conflict(format!(
                "cannot capture a payment session in status {}",
                session.status
            )));
        }

        let provider = self
            .providers
            .get(&session.provider)
            .ok_or_else(|| AppError::not_found("payment provider", &session.provider))?;
        let provider_session_id = session.provider_session_id.clone().ok_or_else(|| {
            AppError::conflict("payment session is missing a provider session id")
        })?;

        // Claim the capture and reserve its amount before calling the provider.
        // The deterministic key lets a retry reuse the provider's idempotency
        // protection, while the timestamp lets a crashed worker's stale claim
        // expire. The reservation is written in the same transaction as the
        // claim, so the cumulative cap holds under concurrency.
        let claimed_at = now_timestamp();
        let claim = match self
            .claim_capture(session_id, requested, idempotency_key, claimed_at)
            .await?
        {
            CaptureClaimOutcome::Claimed(claim) => claim,
            CaptureClaimOutcome::AlreadyCaptured(payment) => return Ok(payment),
        };

        // Provider calls always happen outside database transactions.
        let result = match provider
            .capture(CaptureRequest {
                provider_session_id,
                amount: claim.amount,
                currency_code: session.currency_code.clone(),
                idempotency_key: Some(claim.token.clone()),
                metadata: session.data.clone(),
            })
            .await
        {
            Ok(result) => result,
            Err(error) => {
                let message = error.to_string();
                // Only a structurally invalid request is a definitive failure.
                // Every other error is ambiguous: the capture may have reached
                // the provider even though the response was lost.
                self.record_capture_failure(
                    session_id,
                    &claim,
                    &message,
                    matches!(error, PaymentError::InvalidRequest(_)),
                )
                .await?;
                return Err(error.into());
            }
        };
        self.settle_capture(session_id, &claim, &result).await
    }

    /// Claims one logical capture inside a `BEGIN IMMEDIATE` transaction and
    /// reserves its amount in the session capture ledger.
    ///
    /// The ledger key identifies the logical capture (the caller's idempotency
    /// key, or a derived key for keyless callers). A key that was already
    /// settled returns the recorded payment, a key with a released reservation
    /// resumes the same capture, and a key that is unknown has to pass the
    /// cumulative cap against the reservations of every other capture.
    async fn claim_capture(
        &self,
        session_id: &str,
        requested: Option<i64>,
        idempotency_key: Option<String>,
        claimed_at: i64,
    ) -> Result<CaptureClaimOutcome, AppError> {
        let stale_cutoff = claimed_at.saturating_sub(120);
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let current = load_session(&mut tx, session_id).await?;
        if capture_settled(&current) {
            tx.rollback().await.map_err(database_error)?;
            return Ok(CaptureClaimOutcome::AlreadyCaptured(
                self.payment_for_session(&current).await?,
            ));
        }
        if matches!(
            current.status,
            PaymentStatus::Failed | PaymentStatus::Canceled
        ) {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict(format!(
                "cannot capture a payment session in status {}",
                current.status
            )));
        }

        let mut ledger = CaptureLedger::from_metadata(&current.data);
        let settled_before = ledger.settled_total();
        let amount = match requested {
            Some(amount) => amount,
            None => {
                let remaining = current.amount.saturating_sub(settled_before);
                if remaining <= 0 {
                    tx.rollback().await.map_err(database_error)?;
                    return Err(AppError::conflict(
                        "payment session has no remaining amount to capture",
                    ));
                }
                remaining
            }
        };
        let ledger_key = match normalize_idempotency_key(idempotency_key) {
            Some(key) => key,
            None => {
                keyless_capture_key(current.id.as_str(), settled_before, amount, current.amount)
            }
        };
        match ledger.get(&ledger_key) {
            Some(entry) => {
                if entry.amount != amount {
                    tx.rollback().await.map_err(database_error)?;
                    return Err(AppError::conflict(
                        "payment capture idempotency key was already used for another amount",
                    ));
                }
                if entry.state == CaptureEntryState::Settled {
                    // Same key, already captured: replay the recorded payment.
                    tx.rollback().await.map_err(database_error)?;
                    return Ok(CaptureClaimOutcome::AlreadyCaptured(
                        self.payment_for_session(&current).await?,
                    ));
                }
                if entry.state == CaptureEntryState::Failed
                    && amount > current.amount.saturating_sub(ledger.reserved_total())
                {
                    // The earlier definitive rejection released this
                    // reservation, so a retry has to fit the cap again.
                    tx.rollback().await.map_err(database_error)?;
                    return Err(AppError::validation(
                        "capture amount exceeds the remaining session amount",
                    ));
                }
            }
            None => {
                let remaining = current.amount.saturating_sub(ledger.reserved_total());
                if amount > remaining {
                    tx.rollback().await.map_err(database_error)?;
                    return Err(AppError::validation(
                        "capture amount exceeds the remaining session amount",
                    ));
                }
            }
        }

        let live_claim = current
            .data
            .get(CAPTURE_CLAIM_FIELD)
            .and_then(Value::as_str)
            .zip(
                current
                    .data
                    .get(CAPTURE_CLAIM_AT_FIELD)
                    .and_then(Value::as_i64),
            )
            .is_some_and(|(_, claimed_at)| claimed_at > stale_cutoff);
        if live_claim {
            tx.rollback().await.map_err(database_error)?;
            return Err(AppError::conflict("payment capture is already in progress"));
        }

        let token = capture_provider_key(current.id.as_str(), &ledger_key);
        ledger.insert(
            &ledger_key,
            CaptureEntry {
                amount,
                state: CaptureEntryState::InFlight,
                at: claimed_at,
            },
        );
        let mut data = current.data.clone();
        data.insert(CAPTURE_LEDGER_FIELD, ledger.to_value());
        data.insert(CAPTURE_CLAIM_FIELD, Value::String(token.clone()));
        data.insert(CAPTURE_CLAIM_AT_FIELD, Value::from(claimed_at));
        let claimed = sqlx::query(
            "UPDATE payment_sessions SET data = ?, updated_at = ?
             WHERE id = ?
               AND status IN ('pending', 'requires_action', 'authorized', 'partially_captured')
               AND (json_extract(data, '$.capture_claim') IS NULL
                    OR json_extract(data, '$.capture_claim_at') <= ?)",
        )
        .bind(data.to_json_string())
        .bind(claimed_at)
        .bind(current.id.as_str())
        .bind(stale_cutoff)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if claimed.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            let latest = self.get_session(current.id.as_str()).await?;
            if capture_settled(&latest) {
                return Ok(CaptureClaimOutcome::AlreadyCaptured(
                    self.payment_for_session(&latest).await?,
                ));
            }
            return Err(AppError::conflict("payment capture is already in progress"));
        }
        tx.commit().await.map_err(database_error)?;
        Ok(CaptureClaimOutcome::Claimed(CaptureClaim {
            token,
            ledger_key,
            amount,
        }))
    }

    /// Settles a successful capture: records the captured amount on the
    /// session payment, marks the reservation settled and clears the claim.
    ///
    /// The payment row accumulates the captures of its session, because a
    /// provider (Stripe included) reports one payment object for every capture
    /// of the same session.
    async fn settle_capture(
        &self,
        session_id: &str,
        claim: &CaptureClaim,
        result: &CaptureResult,
    ) -> Result<Payment, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let current = load_session(&mut tx, session_id).await?;
        let mut ledger = CaptureLedger::from_metadata(&current.data);
        ledger.insert(
            &claim.ledger_key,
            CaptureEntry {
                amount: claim.amount,
                state: CaptureEntryState::Settled,
                at: now,
            },
        );
        let captured_total = ledger.settled_total();
        let session_status = if captured_total >= current.amount {
            PaymentStatus::Captured
        } else {
            PaymentStatus::PartiallyCaptured
        };

        let mut data = current.data.clone();
        data.insert(CAPTURE_LEDGER_FIELD, ledger.to_value());
        data.remove(CAPTURE_CLAIM_FIELD);
        data.remove(CAPTURE_CLAIM_AT_FIELD);

        let existing = match current.data.get("payment_id").and_then(Value::as_str) {
            Some(payment_id) => sqlx::query_as::<_, PaymentRow>(
                "SELECT id, order_id, provider, provider_payment_id, amount, currency_code, \
                 status, captured_at, metadata, created_at, updated_at \
                 FROM payments WHERE id = ?",
            )
            .bind(payment_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?,
            None => None,
        };
        let payment = if let Some(row) = existing {
            let mut payment = payment_from_row(row);
            let refunded: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(amount), 0) FROM refunds \
                 WHERE payment_id = ? AND status != 'failed'",
            )
            .bind(payment.id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
            // Recompute from the persisted refunds so a capture that arrives
            // after a refund cannot leave the payment over-refunded.
            payment.status = if refunded >= captured_total {
                PaymentStatus::Refunded
            } else if refunded > 0 {
                PaymentStatus::PartiallyRefunded
            } else {
                session_status
            };
            payment.amount = captured_total;
            if payment.provider_payment_id.is_none() {
                payment.provider_payment_id = Some(result.provider_payment_id.clone());
            }
            payment.captured_at = payment.captured_at.or(Some(now));
            payment.updated_at = now;
            sqlx::query(
                "UPDATE payments SET amount = ?, provider_payment_id = ?, status = ?, \
                 captured_at = ?, updated_at = ? WHERE id = ?",
            )
            .bind(payment.amount)
            .bind(payment.provider_payment_id.as_deref())
            .bind(payment.status.as_str())
            .bind(payment.captured_at)
            .bind(now)
            .bind(payment.id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            payment
        } else {
            let payment = Payment {
                id: PaymentId::new(),
                order_id: current.order_id.clone(),
                provider: current.provider.clone(),
                provider_payment_id: Some(result.provider_payment_id.clone()),
                amount: captured_total,
                currency_code: current.currency_code.clone(),
                status: session_status,
                captured_at: Some(now),
                metadata: payment_metadata(&current.data),
                created_at: now,
                updated_at: now,
            };
            insert_payment(&mut tx, &payment).await?;
            payment
        };
        data.insert("payment_id", Value::String(payment.id.as_str().to_owned()));

        let updated = sqlx::query(
            "UPDATE payment_sessions SET status = ?, data = ?, updated_at = ? \
             WHERE id = ? \
               AND status IN ('pending', 'requires_action', 'authorized', 'partially_captured') \
               AND json_extract(data, '$.capture_claim') = ?",
        )
        .bind(session_status.as_str())
        .bind(data.to_json_string())
        .bind(now)
        .bind(current.id.as_str())
        .bind(&claim.token)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            tx.rollback().await.map_err(database_error)?;
            let latest = self.get_session(current.id.as_str()).await?;
            if capture_settled(&latest) {
                return self.payment_for_session(&latest).await;
            }
            return Err(AppError::conflict("payment capture claim was lost"));
        }
        insert_attempt(
            &mut tx,
            &PaymentAttempt {
                id: PaymentAttemptId::new(),
                payment_session_id: current.id.clone(),
                amount: claim.amount,
                currency_code: current.currency_code.clone(),
                status: PaymentStatus::Captured,
                provider_reference: Some(result.provider_payment_id.clone()),
                error: None,
                created_at: now,
            },
        )
        .await?;
        // Checkout captures before the order exists. In that case a later
        // `assign_order` emits the captured event once the order identifier is
        // known, so consumers never receive a payment event without an order.
        if let Some(order_id) = payment.order_id.as_ref() {
            let complete = session_status == PaymentStatus::Captured;
            append_payment_event(
                &mut tx,
                self.db.store_id(),
                if complete {
                    "payment.captured"
                } else {
                    "payment.partially_captured"
                },
                order_id,
                if complete {
                    payment.amount
                } else {
                    claim.amount
                },
                &payment.currency_code,
                &payment.provider,
                Some(payment.id.as_str()),
                None,
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(payment)
    }

    /// Records a failed capture attempt and releases the claim.
    ///
    /// A definitive rejection releases the reservation as well, because the
    /// provider rejected the capture without side effects; when nothing was
    /// captured yet the session is failed exactly as before. An ambiguous
    /// provider error keeps its reservation counted, so the cumulative cap
    /// cannot be spent twice by a capture that may have succeeded.
    async fn record_capture_failure(
        &self,
        session_id: &str,
        claim: &CaptureClaim,
        message: &str,
        definitive: bool,
    ) -> Result<(), AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let current = load_session(&mut tx, session_id).await?;
        let mut ledger = CaptureLedger::from_metadata(&current.data);
        let settled = ledger.settled_total();
        if definitive {
            ledger.insert(
                &claim.ledger_key,
                CaptureEntry {
                    amount: claim.amount,
                    state: CaptureEntryState::Failed,
                    at: now,
                },
            );
        }
        let fail_session = definitive && settled == 0;
        let mut data = current.data.clone();
        data.insert(CAPTURE_LEDGER_FIELD, ledger.to_value());
        data.insert("last_error", Value::String(message.to_owned()));
        data.remove(CAPTURE_CLAIM_FIELD);
        data.remove(CAPTURE_CLAIM_AT_FIELD);
        let updated = sqlx::query(
            "UPDATE payment_sessions SET status = ?, data = ?, updated_at = ? \
             WHERE id = ? \
               AND status IN ('pending', 'requires_action', 'authorized', 'partially_captured') \
               AND json_extract(data, '$.capture_claim') = ?",
        )
        .bind(if fail_session {
            PaymentStatus::Failed.as_str()
        } else {
            current.status.as_str()
        })
        .bind(data.to_json_string())
        .bind(now)
        .bind(current.id.as_str())
        .bind(&claim.token)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            // A webhook or a stale-claim takeover already reconciled this
            // capture; keep the newer state.
            tx.rollback().await.map_err(database_error)?;
            return Ok(());
        }
        insert_attempt(
            &mut tx,
            &PaymentAttempt {
                id: PaymentAttemptId::new(),
                payment_session_id: current.id.clone(),
                amount: claim.amount,
                currency_code: current.currency_code.clone(),
                status: PaymentStatus::Failed,
                provider_reference: current.provider_session_id.clone(),
                error: Some(message.to_owned()),
                created_at: now,
            },
        )
        .await?;
        if fail_session && let Some(order_id) = current.order_id.as_ref() {
            append_payment_event(
                &mut tx,
                self.db.store_id(),
                "payment.failed",
                order_id,
                claim.amount,
                &current.currency_code,
                &current.provider,
                None,
                Some(message),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}
