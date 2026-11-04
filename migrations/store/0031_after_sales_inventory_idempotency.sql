-- Idempotent inventory movements for after-sales replacements and exchange
-- restocking.
--
-- Each movement carries a deterministic reference derived from the after-sales
-- record and its line. The partial unique indexes turn a repeated or concurrent
-- call into a no-op instead of consuming or returning stock twice.

CREATE UNIQUE INDEX IF NOT EXISTS idx_inventory_movements_after_sales_consume
    ON inventory_movements (reference)
    WHERE reason = 'after_sales_consume' AND reference IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_inventory_movements_exchange_restock
    ON inventory_movements (reference)
    WHERE reason = 'exchange_restock' AND reference IS NOT NULL;
