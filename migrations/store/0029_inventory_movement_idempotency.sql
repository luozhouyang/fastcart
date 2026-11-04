-- Idempotent return restocking.
--
-- Each restock movement carries a deterministic reference derived from the
-- return, order line and cumulative received quantity. The partial unique
-- index makes a repeated or concurrent receive a no-op instead of increasing
-- stock twice.

CREATE UNIQUE INDEX IF NOT EXISTS idx_inventory_movements_return_restock
    ON inventory_movements (reference)
    WHERE reason = 'return_restock' AND reference IS NOT NULL;
