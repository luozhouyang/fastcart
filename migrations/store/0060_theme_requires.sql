-- Themes declare the platform and plugin capabilities they need before
-- activation. Activation checks run after a restart, so the validated
-- requirements are persisted alongside the registration.
--
-- Existing rows keep the column NULL, which reconstructs as no requirements.

ALTER TABLE themes ADD COLUMN requires_json TEXT;
