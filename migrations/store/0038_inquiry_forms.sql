-- Store-scoped inquiry form definitions and immutable published revisions.
--
-- One form has at most one unpublished draft revision. Publishing a revision
-- flips `published_at` and points `inquiry_forms.published_version` at it.
-- Published revisions are never mutated; the next edit creates a new draft.

CREATE TABLE IF NOT EXISTS inquiry_forms (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'published', 'archived')),
    current_version INTEGER NOT NULL DEFAULT 1 CHECK (current_version >= 1),
    published_version INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (published_version IS NULL OR published_version >= 1)
);

CREATE TABLE IF NOT EXISTS inquiry_form_revisions (
    id TEXT PRIMARY KEY,
    form_id TEXT NOT NULL REFERENCES inquiry_forms (id) ON DELETE CASCADE,
    version INTEGER NOT NULL CHECK (version >= 1),
    fields TEXT NOT NULL,
    settings TEXT NOT NULL,
    created_by TEXT,
    created_at INTEGER NOT NULL,
    published_at INTEGER,
    UNIQUE (form_id, version)
);

-- A form may have at most one draft revision at a time.
CREATE UNIQUE INDEX IF NOT EXISTS idx_inquiry_form_revisions_draft
    ON inquiry_form_revisions (form_id)
    WHERE published_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_inquiry_form_revisions_form
    ON inquiry_form_revisions (form_id, version DESC);
