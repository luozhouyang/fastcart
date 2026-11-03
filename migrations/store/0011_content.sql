-- Content: pages, versions, navigation and redirects.

CREATE TABLE IF NOT EXISTS pages (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'draft',
    page_type TEXT NOT NULL DEFAULT 'custom',
    body TEXT NOT NULL DEFAULT '{}',
    seo TEXT NOT NULL DEFAULT '{}',
    metadata TEXT NOT NULL DEFAULT '{}',
    published_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_pages_status ON pages (status);

CREATE TABLE IF NOT EXISTS page_versions (
    id TEXT PRIMARY KEY,
    page_id TEXT NOT NULL REFERENCES pages (id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    body TEXT NOT NULL,
    created_by TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (page_id, version)
);

CREATE TABLE IF NOT EXISTS navigations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS navigation_items (
    id TEXT PRIMARY KEY,
    navigation_id TEXT NOT NULL REFERENCES navigations (id) ON DELETE CASCADE,
    parent_id TEXT REFERENCES navigation_items (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    rank INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_navigation_items_navigation
    ON navigation_items (navigation_id, rank);

CREATE TABLE IF NOT EXISTS redirects (
    id TEXT PRIMARY KEY,
    from_path TEXT NOT NULL UNIQUE,
    to_path TEXT NOT NULL,
    status_code INTEGER NOT NULL DEFAULT 301,
    created_at INTEGER NOT NULL
);
