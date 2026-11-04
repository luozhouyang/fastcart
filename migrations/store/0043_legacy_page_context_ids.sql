-- Repair legacy page block ids that predate the reserved `context` key.
--
-- The SSR runtime injects request context under `data.context`, so a block id
-- `context` would overwrite it and is rejected at write time. Bodies stored by
-- older releases may still contain such an id. Page bodies are written as
-- compact JSON (`serde_json::to_string`), so the exact `"id":"context"` token
-- identifies block ids in both `content` and zone arrays; free-form titles
-- containing quotes are not produced in that spelling. Rename to a stable,
-- non-reserved id so existing pages keep validating and rendering.
UPDATE pages
SET body = replace(body, '"id":"context"', '"id":"context-legacy"')
WHERE body LIKE '%"id":"context"%';
