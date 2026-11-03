-- Durable refund idempotency and provider-reference uniqueness.

ALTER TABLE refunds ADD COLUMN idempotency_key TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_refunds_payment_idempotency
    ON refunds (payment_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_refunds_provider_refund_id
    ON refunds (provider_refund_id)
    WHERE provider_refund_id IS NOT NULL;
