-- Payment collections.
--
-- A collection is the money one checkout or order owes, split across one or
-- more payment sessions. A checkout that only creates a single session still
-- gets a collection, so the shape is the same whether the customer pays with
-- one method or with several.

CREATE TABLE IF NOT EXISTS payment_collections (
    id TEXT PRIMARY KEY,
    checkout_id TEXT,
    order_id TEXT,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- One collection per checkout: `create_collection` is idempotent by the
-- checkout it belongs to.
CREATE UNIQUE INDEX IF NOT EXISTS idx_payment_collections_checkout
    ON payment_collections (checkout_id)
    WHERE checkout_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_payment_collections_order
    ON payment_collections (order_id)
    WHERE order_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS payment_collection_sessions (
    collection_id TEXT NOT NULL REFERENCES payment_collections (id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES payment_sessions (id) ON DELETE CASCADE,
    PRIMARY KEY (collection_id, session_id)
);

-- A session pays into one collection only; a second link would count the same
-- money twice.
CREATE UNIQUE INDEX IF NOT EXISTS idx_payment_collection_sessions_session
    ON payment_collection_sessions (session_id);
