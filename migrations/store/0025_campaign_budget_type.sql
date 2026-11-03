-- Campaign budgets can count usages or monetary spend.

ALTER TABLE campaigns ADD COLUMN budget_type TEXT NOT NULL DEFAULT 'usage';
ALTER TABLE campaigns ADD COLUMN currency_code TEXT;

CREATE INDEX IF NOT EXISTS idx_campaigns_budget_type
    ON campaigns (budget_type, currency_code);
