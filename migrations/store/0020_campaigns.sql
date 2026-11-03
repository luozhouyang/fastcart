-- Campaign budgets and promotion membership.

CREATE TABLE IF NOT EXISTS campaigns (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    budget_limit INTEGER,
    budget_used INTEGER NOT NULL DEFAULT 0,
    starts_at INTEGER,
    ends_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_campaigns_window ON campaigns (starts_at, ends_at);

CREATE TABLE IF NOT EXISTS campaign_promotions (
    campaign_id TEXT NOT NULL REFERENCES campaigns (id) ON DELETE CASCADE,
    promotion_id TEXT NOT NULL REFERENCES promotions (id) ON DELETE CASCADE,
    PRIMARY KEY (campaign_id, promotion_id)
);
