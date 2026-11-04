//! Workflow engine integration tests over real temporary SQLite stores.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use fastcart_infrastructure::{DatabaseOptions, StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{Pagination, StoreId, now_timestamp};
use fastcart_workflows::{
    StepContext, StepError, WorkflowDefinition, WorkflowEngine, WorkflowExecutionFilter,
    WorkflowStep, status,
};
use serde_json::{Value, json};

struct TestStep {
    calls: Arc<AtomicUsize>,
    events: Arc<Mutex<Vec<String>>>,
    fail_until: usize,
    retryable: bool,
    delay_ms: u64,
    compensate_failures: AtomicUsize,
}

impl TestStep {
    fn new(
        calls: &Arc<AtomicUsize>,
        events: &Arc<Mutex<Vec<String>>>,
        fail_until: usize,
        retryable: bool,
        delay_ms: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: Arc::clone(calls),
            events: Arc::clone(events),
            fail_until,
            retryable,
            delay_ms,
            compensate_failures: AtomicUsize::new(0),
        })
    }

    /// Creates a step whose compensation fails the first `failures` times.
    fn with_failing_compensation(
        calls: &Arc<AtomicUsize>,
        events: &Arc<Mutex<Vec<String>>>,
        failures: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            calls: Arc::clone(calls),
            events: Arc::clone(events),
            fail_until: 0,
            retryable: true,
            delay_ms: 0,
            compensate_failures: AtomicUsize::new(failures),
        })
    }
}

#[async_trait]
impl WorkflowStep for TestStep {
    async fn execute(&self, context: &StepContext) -> Result<Value, StepError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.events
            .lock()
            .unwrap()
            .push(format!("execute:{}:{call}", context.step_name));
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        if call <= self.fail_until {
            return Err(if self.retryable {
                StepError::retryable(format!("transient failure {call}"))
            } else {
                StepError::non_retryable(format!("permanent failure {call}"))
            });
        }
        Ok(json!({
            "step": context.step_name,
            "call": call,
            "attempt": context.attempt,
            "input": context.input,
            "state": context.state(),
        }))
    }

    async fn compensate(&self, context: &StepContext, _output: &Value) -> Result<(), StepError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("compensate:{}", context.step_name));
        if self
            .compensate_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(StepError::retryable("compensation provider unavailable"));
        }
        Ok(())
    }
}

async fn store() -> (tempfile::TempDir, StoreDatabase) {
    let dir = tempfile::tempdir().unwrap();
    let manager = StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(4),
    );
    let database = manager.database(&StoreId::new()).await.unwrap();
    (dir, database)
}

async fn seed_execution(
    database: &StoreDatabase,
    id: &str,
    workflow_id: &str,
    idempotency_key: &str,
    status: &str,
    locked_by: Option<&str>,
    locked_until: Option<i64>,
) {
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO workflow_executions
         (id, workflow_id, idempotency_key, status, input, output, error, attempts,
          locked_by, locked_until, started_at, finished_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, '{}', NULL, NULL, 0, ?, ?, ?, NULL, ?, ?)",
    )
    .bind(id)
    .bind(workflow_id)
    .bind(idempotency_key)
    .bind(status)
    .bind(locked_by)
    .bind(locked_until)
    .bind(now.saturating_sub(100))
    .bind(now.saturating_sub(100))
    .bind(now.saturating_sub(100))
    .execute(database.pool())
    .await
    .unwrap();
}

async fn seed_step(
    database: &StoreDatabase,
    execution_id: &str,
    seq: i64,
    name: &str,
    status: &str,
    output: Option<&str>,
    attempts: i64,
) {
    let now = now_timestamp();
    let id = format!("{execution_id}_step_{seq}");
    sqlx::query(
        "INSERT INTO workflow_steps
         (id, execution_id, seq, name, status, input, output, error, attempts,
          compensated_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, '{}', ?, NULL, ?, NULL, ?, ?)",
    )
    .bind(id)
    .bind(execution_id)
    .bind(seq)
    .bind(name)
    .bind(status)
    .bind(output)
    .bind(attempts)
    .bind(now.saturating_sub(100))
    .bind(now.saturating_sub(100))
    .execute(database.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn idempotent_repeat_returns_one_execution_without_rerunning() {
    let (_dir, database) = store().await;
    let engine = WorkflowEngine::new(database);
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register("one", TestStep::new(&calls, &events, 0, true, 0))
        .unwrap();

    let definition = WorkflowDefinition::new("idempotent", ["one"]);
    let first = engine
        .run(
            &definition,
            json!({"order": "o1"}),
            Some("key-1".to_owned()),
        )
        .await
        .unwrap();
    let second = engine
        .run(
            &definition,
            json!({"order": "o1"}),
            Some("key-1".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(first.status, status::COMPLETED);
    assert_eq!(second.status, status::COMPLETED);
    assert_eq!(second.output, first.output);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let page = engine
        .list(&WorkflowExecutionFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(page.count, 1);
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].steps.len(), 1);
    assert_eq!(page.data[0].steps[0].status, status::step::SUCCEEDED);
    assert_eq!(page.data[0].execution.output, page.data[0].steps[0].output);
}

#[tokio::test]
async fn run_resumes_after_a_crash_at_the_first_pending_step() {
    let (_dir, database) = store().await;
    let now = now_timestamp();
    seed_execution(
        &database,
        "wfexec_crash",
        "resumable",
        "resume-key",
        status::RUNNING,
        Some("dead-runner"),
        Some(now.saturating_sub(30)),
    )
    .await;
    seed_step(
        &database,
        "wfexec_crash",
        0,
        "a",
        status::step::SUCCEEDED,
        Some(r#"{"n":1}"#),
        1,
    )
    .await;
    seed_step(
        &database,
        "wfexec_crash",
        1,
        "b",
        status::step::PENDING,
        None,
        0,
    )
    .await;

    let engine = WorkflowEngine::new(database);
    let a_calls = Arc::new(AtomicUsize::new(0));
    let b_calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register("a", TestStep::new(&a_calls, &events, 0, true, 0))
        .unwrap();
    engine
        .register("b", TestStep::new(&b_calls, &events, 0, true, 0))
        .unwrap();

    let execution = engine
        .run(
            &WorkflowDefinition::new("resumable", ["a", "b"]),
            json!({}),
            Some("resume-key".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(execution.id, "wfexec_crash");
    assert_eq!(execution.status, status::COMPLETED);
    assert_eq!(a_calls.load(Ordering::SeqCst), 0, "succeeded step re-ran");
    assert_eq!(b_calls.load(Ordering::SeqCst), 1);

    let detail = engine.get("wfexec_crash").await.unwrap();
    assert_eq!(detail.steps[0].status, status::step::SUCCEEDED);
    assert_eq!(detail.steps[1].status, status::step::SUCCEEDED);
    assert_eq!(
        detail.steps[1].output_json(),
        Some(json!({
            "step": "b",
            "call": 1,
            "attempt": 1,
            "input": {},
            "state": {"a": {"n": 1}},
        }))
    );
}

#[tokio::test]
async fn concurrent_runs_have_a_single_lease_holder() {
    let (_dir, database) = store().await;
    let engine = WorkflowEngine::new(database).with_lease_seconds(30);
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register("slow", TestStep::new(&calls, &events, 0, true, 50))
        .unwrap();

    let definition = WorkflowDefinition::new("concurrent", ["slow"]);
    let first_engine = engine.clone();
    let second_engine = engine.clone();
    let first_definition = definition.clone();
    let second_definition = definition.clone();
    let (first, second) = tokio::join!(
        async move {
            first_engine
                .run(&first_definition, json!({}), Some("shared-key".to_owned()))
                .await
        },
        async move {
            second_engine
                .run(&second_definition, json!({}), Some("shared-key".to_owned()))
                .await
        }
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "both runners advanced the same execution"
    );

    let detail = engine.get(&first.id).await.unwrap();
    assert_eq!(detail.execution.status, status::COMPLETED);
    assert_eq!(detail.steps.len(), 1);
    assert_eq!(detail.steps[0].status, status::step::SUCCEEDED);
}

#[tokio::test]
async fn failure_compensates_succeeded_steps_in_reverse_order() {
    let (_dir, database) = store().await;
    let engine = WorkflowEngine::new(database);
    let a_calls = Arc::new(AtomicUsize::new(0));
    let b_calls = Arc::new(AtomicUsize::new(0));
    let c_calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register("a", TestStep::new(&a_calls, &events, 0, true, 0))
        .unwrap();
    engine
        .register("b", TestStep::new(&b_calls, &events, 0, true, 0))
        .unwrap();
    engine
        .register("c", TestStep::new(&c_calls, &events, 1, false, 0))
        .unwrap();

    let execution = engine
        .run(
            &WorkflowDefinition::new("compensating", ["a", "b", "c"]),
            json!({}),
            None,
        )
        .await
        .unwrap();

    assert_eq!(execution.status, status::FAILED);
    assert!(execution.error.is_some());
    assert_eq!(
        c_calls.load(Ordering::SeqCst),
        1,
        "non-retryable error retried"
    );

    let detail = engine.get(&execution.id).await.unwrap();
    assert_eq!(detail.steps[0].status, status::step::COMPENSATED);
    assert_eq!(detail.steps[1].status, status::step::COMPENSATED);
    assert_eq!(detail.steps[2].status, status::step::FAILED);
    assert!(detail.steps[0].compensated_at.is_some());
    assert!(detail.steps[1].compensated_at.is_some());

    let compensations: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.starts_with("compensate:"))
        .cloned()
        .collect();
    assert_eq!(compensations, vec!["compensate:b", "compensate:a"]);
}

#[tokio::test]
async fn retryable_failures_are_retried_and_attempts_are_persisted() {
    let (_dir, database) = store().await;
    let engine = WorkflowEngine::new(database).with_max_attempts(3);
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register("flaky", TestStep::new(&calls, &events, 2, true, 0))
        .unwrap();

    let execution = engine
        .run(
            &WorkflowDefinition::new("retries", ["flaky"]),
            json!({}),
            None,
        )
        .await
        .unwrap();

    assert_eq!(execution.status, status::COMPLETED);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let detail = engine.get(&execution.id).await.unwrap();
    assert_eq!(detail.steps[0].attempts, 3);
    assert_eq!(detail.steps[0].status, status::step::SUCCEEDED);
    assert_eq!(detail.execution.attempts, 3);

    let attempts: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.starts_with("execute:flaky"))
        .cloned()
        .collect();
    assert_eq!(
        attempts,
        vec!["execute:flaky:1", "execute:flaky:2", "execute:flaky:3"]
    );
}

#[tokio::test]
async fn expired_leases_can_be_reclaimed() {
    let (_dir, database) = store().await;
    let now = now_timestamp();
    seed_execution(
        &database,
        "wfexec_stale",
        "reclaim",
        "reclaim-key",
        status::RUNNING,
        Some("stopped-runner"),
        Some(now.saturating_sub(1)),
    )
    .await;
    seed_step(
        &database,
        "wfexec_stale",
        0,
        "a",
        status::step::PENDING,
        None,
        0,
    )
    .await;
    seed_step(
        &database,
        "wfexec_stale",
        1,
        "b",
        status::step::PENDING,
        None,
        0,
    )
    .await;

    let engine = WorkflowEngine::new(database);
    let a_calls = Arc::new(AtomicUsize::new(0));
    let b_calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register("a", TestStep::new(&a_calls, &events, 0, true, 0))
        .unwrap();
    engine
        .register("b", TestStep::new(&b_calls, &events, 0, true, 0))
        .unwrap();

    let execution = engine
        .run(
            &WorkflowDefinition::new("reclaim", ["a", "b"]),
            json!({}),
            Some("reclaim-key".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(execution.id, "wfexec_stale");
    assert_eq!(execution.status, status::COMPLETED);
    assert_eq!(a_calls.load(Ordering::SeqCst), 1);
    assert_eq!(b_calls.load(Ordering::SeqCst), 1);
    assert!(execution.locked_by.is_none());
    assert!(execution.locked_until.is_none());
}

#[tokio::test]
async fn failed_compensation_is_retried_on_a_resumed_drive() {
    let (_dir, database) = store().await;
    let engine = WorkflowEngine::new(database).with_max_attempts(3);
    let a_calls = Arc::new(AtomicUsize::new(0));
    let b_calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register(
            "a",
            TestStep::with_failing_compensation(&a_calls, &events, 1),
        )
        .unwrap();
    engine
        .register("b", TestStep::new(&b_calls, &events, 1, false, 0))
        .unwrap();

    let definition = WorkflowDefinition::new("resumable-compensation", ["a", "b"]);
    let key = Some("resume-compensation".to_owned());

    // First drive: step b fails, step a's compensation provider is down.
    let first = engine
        .run(&definition, json!({}), key.clone())
        .await
        .unwrap();
    assert_eq!(
        first.status,
        status::PENDING,
        "prematurely finalized a failed compensation"
    );
    let detail = engine.get(&first.id).await.unwrap();
    assert_eq!(detail.steps[0].status, status::step::COMPENSATION_FAILED);
    assert_eq!(detail.steps[0].compensation_attempts, 1);
    assert_eq!(detail.steps[1].status, status::step::FAILED);

    // Resuming retries the failed compensation instead of skipping it.
    let second = engine.run(&definition, json!({}), key).await.unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.status, status::FAILED);
    let detail = engine.get(&first.id).await.unwrap();
    assert_eq!(detail.steps[0].status, status::step::COMPENSATED);
    assert_eq!(detail.steps[0].compensation_attempts, 2);
    assert!(detail.steps[0].compensated_at.is_some());
    assert_eq!(detail.steps[1].status, status::step::FAILED);

    let compensations: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.starts_with("compensate:"))
        .cloned()
        .collect();
    assert_eq!(compensations, vec!["compensate:a", "compensate:a"]);
}

#[tokio::test]
async fn compensation_retries_stop_at_the_attempt_budget() {
    let (_dir, database) = store().await;
    let engine = WorkflowEngine::new(database).with_max_attempts(2);
    let a_calls = Arc::new(AtomicUsize::new(0));
    let b_calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register(
            "a",
            TestStep::with_failing_compensation(&a_calls, &events, usize::MAX),
        )
        .unwrap();
    engine
        .register("b", TestStep::new(&b_calls, &events, 1, false, 0))
        .unwrap();

    let definition = WorkflowDefinition::new("exhausted-compensation", ["a", "b"]);
    let key = Some("exhausted-compensation".to_owned());

    let first = engine
        .run(&definition, json!({}), key.clone())
        .await
        .unwrap();
    assert_eq!(first.status, status::PENDING);

    let second = engine.run(&definition, json!({}), key).await.unwrap();
    assert_eq!(second.status, status::FAILED);
    let detail = engine.get(&first.id).await.unwrap();
    assert_eq!(detail.steps[0].status, status::step::COMPENSATION_FAILED);
    assert_eq!(detail.steps[0].compensation_attempts, 2);

    let compensations = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.starts_with("compensate:"))
        .count();
    assert_eq!(compensations, 2, "retried past the attempt budget");
}

#[tokio::test]
async fn duplicate_step_registration_is_rejected() {
    let (_dir, database) = store().await;
    let engine = WorkflowEngine::new(database);
    let calls = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    engine
        .register("a", TestStep::new(&calls, &events, 0, true, 0))
        .unwrap();
    let error = engine
        .register("a", TestStep::new(&calls, &events, 0, true, 0))
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
