-- Recorded AI agent tasks.
--
-- Tasks are metadata in this slice: an invocation is recorded and moved
-- through the state machine defined by fastcart-ai, but no model or tool call
-- is executed. `tool` holds a stable registry name, `input_json` the recorded
-- invocation input and `status` the snake_case AgentTaskStatus value.
-- Transitions are guarded by the server and written to the admin audit log.

CREATE TABLE IF NOT EXISTS ai_tasks (
    id TEXT PRIMARY KEY,
    tool TEXT NOT NULL,
    input_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL,
    error TEXT,
    created_by TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ai_tasks_created_at
    ON ai_tasks (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_ai_tasks_status
    ON ai_tasks (status);
