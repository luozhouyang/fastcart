//! Collection and refund of claim and exchange price differences.

//! The workflow owns the cross-module idempotency boundary. Provider refunds
//! use a stable key per after-sales record, order refunds use the same key as a
//! local idempotency marker, and restock movements use a deterministic
//! reference. Replaying any step therefore never moves money or stock twice.

use fastcart_kernel::{AppError, ClaimId, ExchangeId, OrderId};
use fastcart_module_order::{ClaimStatus, ExchangeStatus};

use super::*;
impl AfterSalesWorkflow {
    /// Opens the money an accepted exchange has the customer owing.
    ///
    /// Accepting a dearer exchange settles the difference on the order; this
    /// makes it payable. The order's collection grows by the difference and a
    /// session pays the increase — nothing is charged here, the customer pays
    /// that session like any other. Idempotent: replaying it returns the
    /// session that already collects the difference.
    pub async fn collect_exchange_difference(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
        provider: &str,
    ) -> Result<DifferenceCollectionResult, AppError> {
        let exchange = self.order.get_exchange(order_id, exchange_id).await?;
        if exchange.status != ExchangeStatus::Accepted {
            return Err(AppError::conflict(
                "an exchange must be accepted before its difference can be collected",
            ));
        }
        let difference = exchange.difference_total.ok_or_else(|| {
            AppError::conflict("the accepted exchange settled no price difference")
        })?;
        self.collect_difference(
            order_id,
            difference,
            EXCHANGE_DIFFERENCE_REFUND_PREFIX,
            exchange_id.as_str(),
            provider,
        )
        .await
    }

    /// Opens the money an accepted claim has the customer owing.
    ///
    /// See [`Self::collect_exchange_difference`]: same settlement, same payable.
    pub async fn collect_claim_difference(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
        provider: &str,
    ) -> Result<DifferenceCollectionResult, AppError> {
        let claim = self.order.get_claim(order_id, claim_id).await?;
        if claim.status != ClaimStatus::Accepted {
            return Err(AppError::conflict(
                "a claim must be accepted before its difference can be collected",
            ));
        }
        let difference = claim
            .difference_total
            .ok_or_else(|| AppError::conflict("the accepted claim settled no price difference"))?;
        self.collect_difference(
            order_id,
            difference,
            CLAIM_DIFFERENCE_REFUND_PREFIX,
            claim_id.as_str(),
            provider,
        )
        .await
    }

    /// Refunds the difference an accepted exchange owes the customer.
    ///
    /// Accepting an exchange settles the difference on the order — a cheaper
    /// replacement leaves the customer owed money — but paying it back is the
    /// merchant's decision, so it is its own call rather than a side effect of
    /// accepting. Idempotent: replaying it pays once.
    pub async fn refund_exchange_difference(
        &self,
        order_id: &OrderId,
        exchange_id: &ExchangeId,
    ) -> Result<DifferenceRefundResult, AppError> {
        let exchange = self.order.get_exchange(order_id, exchange_id).await?;
        if exchange.status != ExchangeStatus::Accepted {
            return Err(AppError::conflict(
                "an exchange must be accepted before its difference can be refunded",
            ));
        }
        let difference = exchange.difference_total.ok_or_else(|| {
            AppError::conflict("the accepted exchange settled no price difference")
        })?;
        self.refund_difference(
            order_id,
            difference,
            EXCHANGE_DIFFERENCE_REFUND_PREFIX,
            exchange_id.as_str(),
            exchange.reason.clone(),
        )
        .await
    }

    /// Refunds the difference an accepted claim owes the customer.
    ///
    /// See [`Self::refund_exchange_difference`]: same settlement, same
    /// one-call, idempotent payout.
    pub async fn refund_claim_difference(
        &self,
        order_id: &OrderId,
        claim_id: &ClaimId,
    ) -> Result<DifferenceRefundResult, AppError> {
        let claim = self.order.get_claim(order_id, claim_id).await?;
        if claim.status != ClaimStatus::Accepted {
            return Err(AppError::conflict(
                "a claim must be accepted before its difference can be refunded",
            ));
        }
        let difference = claim
            .difference_total
            .ok_or_else(|| AppError::conflict("the accepted claim settled no price difference"))?;
        self.refund_difference(
            order_id,
            difference,
            CLAIM_DIFFERENCE_REFUND_PREFIX,
            claim_id.as_str(),
            claim.reason.clone(),
        )
        .await
    }
}
