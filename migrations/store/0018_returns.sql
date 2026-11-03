-- Post-purchase operations: returns, claims, exchanges and order edits.
--
-- Historical `order_items` rows are immutable snapshots and are never rewritten
-- by these flows. Every change is recorded in its own table and the order
-- aggregate exposes the records read-only.

CREATE TABLE IF NOT EXISTS order_returns (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'draft',
    reason TEXT,
    refund_id TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    requested_at INTEGER,
    received_at INTEGER,
    written_off_at INTEGER,
    canceled_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_order_returns_order
    ON order_returns (order_id, status, created_at);
CREATE INDEX IF NOT EXISTS idx_order_returns_refund
    ON order_returns (refund_id);

CREATE TABLE IF NOT EXISTS order_return_items (
    id TEXT PRIMARY KEY,
    return_id TEXT NOT NULL REFERENCES order_returns (id) ON DELETE CASCADE,
    order_item_id TEXT NOT NULL REFERENCES order_items (id) ON DELETE RESTRICT,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    received_quantity INTEGER NOT NULL DEFAULT 0 CHECK (received_quantity >= 0),
    written_off_quantity INTEGER NOT NULL DEFAULT 0 CHECK (written_off_quantity >= 0),
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (received_quantity <= quantity),
    CHECK (written_off_quantity <= received_quantity)
);

CREATE INDEX IF NOT EXISTS idx_order_return_items_return
    ON order_return_items (return_id, status);
CREATE INDEX IF NOT EXISTS idx_order_return_items_order_item
    ON order_return_items (order_item_id);

CREATE TABLE IF NOT EXISTS order_claims (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'requested',
    reason TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    accepted_at INTEGER,
    canceled_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_order_claims_order
    ON order_claims (order_id, status, created_at);

CREATE TABLE IF NOT EXISTS order_claim_items (
    id TEXT PRIMARY KEY,
    claim_id TEXT NOT NULL REFERENCES order_claims (id) ON DELETE CASCADE,
    order_item_id TEXT NOT NULL REFERENCES order_items (id) ON DELETE RESTRICT,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    item_type TEXT NOT NULL DEFAULT 'refund',
    replacement_variant_id TEXT,
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_claim_items_claim
    ON order_claim_items (claim_id, status);
CREATE INDEX IF NOT EXISTS idx_order_claim_items_order_item
    ON order_claim_items (order_item_id);

CREATE TABLE IF NOT EXISTS order_exchanges (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'requested',
    reason TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    accepted_at INTEGER,
    canceled_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_order_exchanges_order
    ON order_exchanges (order_id, status, created_at);

CREATE TABLE IF NOT EXISTS order_exchange_items (
    id TEXT PRIMARY KEY,
    exchange_id TEXT NOT NULL REFERENCES order_exchanges (id) ON DELETE CASCADE,
    order_item_id TEXT NOT NULL REFERENCES order_items (id) ON DELETE RESTRICT,
    direction TEXT NOT NULL,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    replacement_variant_id TEXT,
    reason TEXT,
    status TEXT NOT NULL DEFAULT 'requested',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_exchange_items_exchange
    ON order_exchange_items (exchange_id, direction, status);
CREATE INDEX IF NOT EXISTS idx_order_exchange_items_order_item
    ON order_exchange_items (order_item_id);

CREATE TABLE IF NOT EXISTS order_edits (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'draft',
    reason TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    before_subtotal INTEGER NOT NULL DEFAULT 0,
    after_subtotal INTEGER NOT NULL DEFAULT 0,
    before_total INTEGER NOT NULL DEFAULT 0,
    after_total INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    confirmed_at INTEGER,
    canceled_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_order_edits_order
    ON order_edits (order_id, status, created_at);

CREATE TABLE IF NOT EXISTS order_edit_items (
    id TEXT PRIMARY KEY,
    edit_id TEXT NOT NULL REFERENCES order_edits (id) ON DELETE CASCADE,
    order_item_id TEXT REFERENCES order_items (id) ON DELETE RESTRICT,
    action TEXT NOT NULL,
    product_id TEXT,
    variant_id TEXT,
    product_title TEXT NOT NULL,
    variant_title TEXT,
    sku TEXT,
    unit_price INTEGER NOT NULL DEFAULT 0,
    before_quantity INTEGER NOT NULL DEFAULT 0,
    after_quantity INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'draft',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (before_quantity >= 0),
    CHECK (after_quantity >= 0)
);

CREATE INDEX IF NOT EXISTS idx_order_edit_items_edit
    ON order_edit_items (edit_id, status);
CREATE INDEX IF NOT EXISTS idx_order_edit_items_order_item
    ON order_edit_items (order_item_id);
