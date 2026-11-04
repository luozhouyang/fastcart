-- Inquiry submissions and private attachment uploads.
--
-- Core contact fields are mirrored as columns for filtering and notification
-- rendering. Custom fields live in the `data` JSON object. Every submission
-- references the immutable form revision it was accepted against.

CREATE TABLE IF NOT EXISTS inquiries (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE,
    form_id TEXT NOT NULL REFERENCES inquiry_forms (id) ON DELETE RESTRICT,
    form_revision_id TEXT NOT NULL REFERENCES inquiry_form_revisions (id) ON DELETE RESTRICT,
    form_version INTEGER NOT NULL CHECK (form_version >= 1),
    status TEXT NOT NULL DEFAULT 'new'
        CHECK (status IN ('new', 'read', 'replied', 'closed', 'spam')),
    name TEXT,
    email TEXT,
    phone TEXT,
    subject TEXT,
    message TEXT,
    data TEXT NOT NULL DEFAULT '{}',
    source TEXT NOT NULL DEFAULT 'contact_form',
    page_url TEXT,
    locale TEXT,
    user_agent TEXT,
    ip_hash TEXT,
    idempotency_key TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (form_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_inquiries_status
    ON inquiries (status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_inquiries_form
    ON inquiries (form_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_inquiries_email
    ON inquiries (email);
CREATE INDEX IF NOT EXISTS idx_inquiries_created
    ON inquiries (created_at DESC);

CREATE TABLE IF NOT EXISTS inquiry_files (
    id TEXT PRIMARY KEY,
    form_id TEXT NOT NULL REFERENCES inquiry_forms (id) ON DELETE CASCADE,
    inquiry_id TEXT REFERENCES inquiries (id) ON DELETE CASCADE,
    field_key TEXT NOT NULL DEFAULT 'attachments',
    storage_key TEXT NOT NULL UNIQUE,
    original_name TEXT,
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes > 0),
    checksum_sha256 TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'claimed', 'expired')),
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    claimed_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_inquiry_files_pending
    ON inquiry_files (status, expires_at);
CREATE INDEX IF NOT EXISTS idx_inquiry_files_inquiry
    ON inquiry_files (inquiry_id, field_key);
