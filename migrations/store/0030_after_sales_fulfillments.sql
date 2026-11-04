-- Provider-side replacements for accepted claims and exchanges.
--
-- Replacement fulfillment is intentionally separate from `order_fulfillments`:
-- it records goods shipped in response to an after-sales decision, not the
-- original order fulfillment. A row belongs to exactly one claim or exchange;
-- the partial unique indexes make replaying a fulfillment idempotent.

CREATE TABLE IF NOT EXISTS after_sales_fulfillments (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    claim_id TEXT REFERENCES order_claims (id) ON DELETE CASCADE,
    exchange_id TEXT REFERENCES order_exchanges (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'fulfilled',
    provider_id TEXT NOT NULL,
    provider_fulfillment_id TEXT,
    provider_data TEXT NOT NULL DEFAULT '{}',
    tracking_numbers TEXT NOT NULL DEFAULT '[]',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK ((claim_id IS NULL) <> (exchange_id IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_after_sales_fulfillments_order
    ON after_sales_fulfillments (order_id, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS idx_after_sales_fulfillments_claim
    ON after_sales_fulfillments (claim_id)
    WHERE claim_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_after_sales_fulfillments_exchange
    ON after_sales_fulfillments (exchange_id)
    WHERE exchange_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS after_sales_fulfillment_items (
    id TEXT PRIMARY KEY,
    fulfillment_id TEXT NOT NULL
        REFERENCES after_sales_fulfillments (id) ON DELETE CASCADE,
    variant_id TEXT NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_after_sales_fulfillment_items_fulfillment
    ON after_sales_fulfillment_items (fulfillment_id, variant_id);
