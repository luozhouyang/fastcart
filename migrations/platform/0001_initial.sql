-- Platform database: global installation state.
-- Store-scoped commerce data never lives here.

CREATE TABLE IF NOT EXISTS stores (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'provisioning',
    database_key TEXT NOT NULL UNIQUE,
    primary_domain TEXT,
    locale TEXT NOT NULL DEFAULT 'en-US',
    currency_code TEXT NOT NULL DEFAULT 'USD',
    timezone TEXT NOT NULL DEFAULT 'UTC',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_stores_status ON stores (status);

CREATE TABLE IF NOT EXISTS store_domains (
    id TEXT PRIMARY KEY,
    store_id TEXT NOT NULL REFERENCES stores (id) ON DELETE CASCADE,
    hostname TEXT NOT NULL UNIQUE,
    is_primary INTEGER NOT NULL DEFAULT 0,
    verified_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_store_domains_store ON store_domains (store_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_store_domains_primary
    ON store_domains (store_id) WHERE is_primary = 1;

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    first_name TEXT,
    last_name TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    email_verified_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL DEFAULT 'emailpass',
    password_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (user_id, provider_id)
);

CREATE TABLE IF NOT EXISTS admin_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    ip_address TEXT,
    user_agent TEXT
);

CREATE INDEX IF NOT EXISTS idx_admin_sessions_user ON admin_sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_admin_sessions_expiry ON admin_sessions (expires_at);

CREATE TABLE IF NOT EXISTS store_memberships (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    store_id TEXT NOT NULL REFERENCES stores (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (user_id, store_id)
);

CREATE INDEX IF NOT EXISTS idx_store_memberships_store ON store_memberships (store_id);

CREATE TABLE IF NOT EXISTS roles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    is_system INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS permissions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT
);

CREATE TABLE IF NOT EXISTS role_permissions (
    role_id TEXT NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    permission_id TEXT NOT NULL REFERENCES permissions (id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
);

CREATE TABLE IF NOT EXISTS membership_roles (
    membership_id TEXT NOT NULL REFERENCES store_memberships (id) ON DELETE CASCADE,
    role_id TEXT NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    PRIMARY KEY (membership_id, role_id)
);

CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    store_id TEXT REFERENCES stores (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    scopes TEXT NOT NULL DEFAULT '[]',
    expires_at INTEGER,
    revoked_at INTEGER,
    last_used_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens (user_id);

CREATE TABLE IF NOT EXISTS platform_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS store_provisioning_jobs (
    id TEXT PRIMARY KEY,
    store_id TEXT NOT NULL REFERENCES stores (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending',
    current_step TEXT,
    steps TEXT NOT NULL DEFAULT '{}',
    last_error TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_provisioning_status ON store_provisioning_jobs (status);

CREATE TABLE IF NOT EXISTS platform_audit_logs (
    id TEXT PRIMARY KEY,
    actor_id TEXT,
    store_id TEXT,
    action TEXT NOT NULL,
    resource_type TEXT,
    resource_id TEXT,
    data TEXT NOT NULL DEFAULT '{}',
    ip_address TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_store ON platform_audit_logs (store_id, created_at);
CREATE INDEX IF NOT EXISTS idx_audit_actor ON platform_audit_logs (actor_id, created_at);

CREATE TABLE IF NOT EXISTS platform_jobs (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending',
    priority INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    available_at INTEGER NOT NULL,
    locked_at INTEGER,
    locked_by TEXT,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_platform_jobs_claim
    ON platform_jobs (status, available_at, priority);

CREATE TABLE IF NOT EXISTS platform_outbox (
    id TEXT PRIMARY KEY,
    store_id TEXT,
    event_type TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    payload TEXT NOT NULL,
    idempotency_key TEXT,
    created_at INTEGER NOT NULL,
    processed_at INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_retry_at INTEGER,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_platform_outbox_pending
    ON platform_outbox (processed_at, next_retry_at);

-- Seeded permissions. Names are stable API contracts.
INSERT OR IGNORE INTO permissions (id, name, description) VALUES
    ('perm_products_read', 'products.read', 'Read products'),
    ('perm_products_write', 'products.write', 'Create and modify products'),
    ('perm_orders_read', 'orders.read', 'Read orders'),
    ('perm_orders_write', 'orders.write', 'Modify orders'),
    ('perm_orders_cancel', 'orders.cancel', 'Cancel orders'),
    ('perm_customers_read', 'customers.read', 'Read customers'),
    ('perm_customers_write', 'customers.write', 'Modify customers'),
    ('perm_inventory_read', 'inventory.read', 'Read inventory'),
    ('perm_inventory_write', 'inventory.write', 'Adjust and reserve inventory'),
    ('perm_promotions_read', 'promotions.read', 'Read promotions'),
    ('perm_promotions_write', 'promotions.write', 'Create and modify promotions'),
    ('perm_settings_read', 'settings.read', 'Read store settings'),
    ('perm_settings_write', 'settings.write', 'Modify store settings'),
    ('perm_themes_read', 'themes.read', 'Read themes and content'),
    ('perm_themes_write', 'themes.write', 'Modify themes and content'),
    ('perm_users_read', 'users.read', 'Read store users'),
    ('perm_users_write', 'users.write', 'Invite and modify store users'),
    ('perm_payments_read', 'payments.read', 'Read payments'),
    ('perm_payments_write', 'payments.write', 'Capture and refund payments');

INSERT OR IGNORE INTO roles (id, name, description, is_system) VALUES
    ('role_owner', 'owner', 'Full control of the store', 1),
    ('role_admin', 'admin', 'Administrative access to the store', 1),
    ('role_manager', 'manager', 'Operational access to the store', 1),
    ('role_product_manager', 'product-manager', 'Catalog and inventory access', 1),
    ('role_order_manager', 'order-manager', 'Order and customer access', 1),
    ('role_content_editor', 'content-editor', 'Content and theme access', 1),
    ('role_support_agent', 'support-agent', 'Customer support access', 1);

-- Owner and admin hold every permission.
INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
    SELECT r.id, p.id FROM roles r CROSS JOIN permissions p
    WHERE r.id IN ('role_owner', 'role_admin');

INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
    SELECT 'role_manager', id FROM permissions
    WHERE name NOT IN ('users.write', 'users.read');

INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
    SELECT 'role_product_manager', id FROM permissions
    WHERE name IN (
        'products.read', 'products.write', 'inventory.read', 'inventory.write',
        'promotions.read', 'promotions.write', 'settings.read'
    );

INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
    SELECT 'role_order_manager', id FROM permissions
    WHERE name IN (
        'orders.read', 'orders.write', 'orders.cancel', 'customers.read',
        'customers.write', 'inventory.read', 'inventory.write', 'payments.read',
        'payments.write'
    );

INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
    SELECT 'role_content_editor', id FROM permissions
    WHERE name IN ('themes.read', 'themes.write', 'settings.read', 'products.read');

INSERT OR IGNORE INTO role_permissions (role_id, permission_id)
    SELECT 'role_support_agent', id FROM permissions
    WHERE name IN (
        'orders.read', 'customers.read', 'customers.write', 'products.read',
        'inventory.read', 'settings.read'
    );
