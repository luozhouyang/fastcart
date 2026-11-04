-- Persistent workflow executions and per-step checkpoints.
--
-- Executions are leased like background jobs: a runner must hold `locked_by`
-- until it advances, and an expired lease may be reclaimed by another runner.
-- Step rows capture enough state to resume after a crash and to compensate
-- already-succeeded steps in reverse order.

CREATE TABLE IF NOT EXISTS workflow_executions (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    idempotency_key TEXT,
    status TEXT NOT NULL,
    input TEXT NOT NULL,
    output TEXT,
    error TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    locked_by TEXT,
    locked_until INTEGER,
    started_at INTEGER,
    finished_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_executions_idempotency
    ON workflow_executions (workflow_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_workflow_executions_lease
    ON workflow_executions (status, locked_until);

CREATE INDEX IF NOT EXISTS idx_workflow_executions_workflow
    ON workflow_executions (workflow_id, created_at);

CREATE TABLE IF NOT EXISTS workflow_steps (
    id TEXT PRIMARY KEY,
    execution_id TEXT NOT NULL REFERENCES workflow_executions (id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    input TEXT NOT NULL DEFAULT '{}',
    output TEXT,
    error TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    compensation_attempts INTEGER NOT NULL DEFAULT 0,
    compensated_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (execution_id, seq)
);
