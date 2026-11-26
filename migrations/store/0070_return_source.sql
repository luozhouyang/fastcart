-- Record who created a return.
--
-- Rows that predate this migration were created by store administrators and
-- read as `admin` through the column default. Customer self-service returns are
-- written as `customer` inside the same transaction that creates the return, so
-- the admin list can distinguish them without inspecting free-form metadata.

ALTER TABLE order_returns ADD COLUMN source TEXT NOT NULL DEFAULT 'admin';
