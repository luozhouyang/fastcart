-- Indexes for queries introduced with capability bindings and page routing.
--
-- Grant resolution and binding validation filter enabled bindings and group
-- them by capability id, and event dispatch fans out by event type. Page
-- routing matches the effective published type, which the existing status
-- index cannot serve.

CREATE INDEX IF NOT EXISTS idx_store_capability_bindings_enabled
    ON store_capability_bindings (enabled, capability_id);

CREATE INDEX IF NOT EXISTS idx_plugin_event_subscriptions_type
    ON plugin_event_subscriptions (event_type, enabled);

CREATE INDEX IF NOT EXISTS idx_pages_effective_type
    ON pages (COALESCE(published_page_type, page_type))
    WHERE deleted_at IS NULL;
