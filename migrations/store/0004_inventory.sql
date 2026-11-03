-- Inventory: locations, items, levels, reservations and movements.

CREATE TABLE IF NOT EXISTS inventory_locations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    address TEXT NOT NULL DEFAULT '{}',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS inventory_items (
    id TEXT PRIMARY KEY,
    sku TEXT UNIQUE,
    variant_id TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_inventory_items_variant
    ON inventory_items (variant_id) WHERE variant_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS inventory_levels (
    id TEXT PRIMARY KEY,
    inventory_item_id TEXT NOT NULL REFERENCES inventory_items (id) ON DELETE CASCADE,
    location_id TEXT NOT NULL REFERENCES inventory_locations (id) ON DELETE CASCADE,
    stocked_quantity INTEGER NOT NULL DEFAULT 0,
    reserved_quantity INTEGER NOT NULL DEFAULT 0,
    incoming_quantity INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (inventory_item_id, location_id)
);

CREATE INDEX IF NOT EXISTS idx_inventory_levels_location
    ON inventory_levels (location_id);

CREATE TABLE IF NOT EXISTS inventory_reservations (
    id TEXT PRIMARY KEY,
    inventory_item_id TEXT NOT NULL REFERENCES inventory_items (id) ON DELETE CASCADE,
    location_id TEXT NOT NULL REFERENCES inventory_locations (id) ON DELETE CASCADE,
    quantity INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'reserved',
    line_item_id TEXT,
    checkout_id TEXT,
    order_id TEXT,
    expires_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_reservations_status
    ON inventory_reservations (status, expires_at);
CREATE INDEX IF NOT EXISTS idx_reservations_item
    ON inventory_reservations (inventory_item_id);
CREATE INDEX IF NOT EXISTS idx_reservations_checkout
    ON inventory_reservations (checkout_id);

CREATE TABLE IF NOT EXISTS inventory_movements (
    id TEXT PRIMARY KEY,
    inventory_item_id TEXT NOT NULL REFERENCES inventory_items (id) ON DELETE CASCADE,
    location_id TEXT NOT NULL REFERENCES inventory_locations (id) ON DELETE CASCADE,
    quantity_delta INTEGER NOT NULL,
    reason TEXT NOT NULL,
    reference TEXT,
    actor_id TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_inventory_movements_item
    ON inventory_movements (inventory_item_id, created_at);
