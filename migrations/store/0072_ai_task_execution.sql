-- Tool execution state for recorded AI agent tasks.
--
-- Read-only tools execute immediately when the task is created. Mutating tools
-- stay `pending` until an administrator approves them; approval claims the task
-- (`pending` -> `running`) and executes it after the claim commits. The runner
-- then writes `succeeded` or `failed` in a guarded update that only matches the
-- `running` row, so concurrent approvals cannot execute a task twice.
--
-- `output_json` holds the successful tool result. `started_at` is set when the
-- task is claimed (or re-claimed after an interrupted run) and `finished_at`
-- when a terminal status is written. Both are Unix seconds.

ALTER TABLE ai_tasks ADD COLUMN output_json TEXT;
ALTER TABLE ai_tasks ADD COLUMN started_at INTEGER;
ALTER TABLE ai_tasks ADD COLUMN finished_at INTEGER;
