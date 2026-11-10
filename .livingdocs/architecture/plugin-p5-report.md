# Plugin System P5 Report: Events and Jobs

**Date:** 2026-11-10
**Status:** Implemented. Admin UI surfacing and bulk replay remain follow-ups.
**Companion documents:** [Plugin System design](workerd-plugin-system.md), [P2 report](plugin-p2-report.md), [P6 report](plugin-p6-report.md).

---

## 1. Data and migrations

`migrations/store/0049_plugin_dispatch.sql` adds delivery/job leases:

- `plugin_deliveries.locked_at`, `locked_by`;
- `plugin_jobs.locked_at`, `locked_by`, `max_attempts`;
- claim indexes for both queues.

The tables themselves were created in `0048_plugins.sql`.

## 2. Subscription sync

`PluginService` now:

- inserts one `plugin_event_subscriptions` row for every `capabilities.events` entry on install;
- replaces the set on upgrade;
- deletes subscriptions, deliveries and queued jobs on remove.

## 3. Dispatcher (`crates/plugin/src/dispatch/`)

`PluginDispatch::run(store, limit)` performs one bounded pass:

1. **Fan-out** — reads pending `outbox` rows, finds enabled subscribers per event type, and inserts idempotent `plugin_deliveries` rows (`UNIQUE(plugin_id, event_id)`).
2. **Deliveries** — claims due `pending` rows with a `running` lease, mints an `actor.kind=event` context token, invokes `POST /events` with `{event, attempt}`, and:
   - 2xx → `done`;
   - 4xx → `dead`;
   - 5xx/timeout → retry with `5 * 2^attempt` backoff (capped) and `dead` after five attempts;
   - stale `running` leases are returned to `pending`.
3. **Scheduling** — for every enabled manifest job with a five-field cron schedule, inserts a deterministic `plugin_jobs` row keyed by `(plugin, job, minute)`.
4. **Jobs** — claims due jobs, mints an `actor.kind=job` token, invokes `POST /jobs/{name}`, and applies the same success/retry/failed policy.

Cron matching is a small UTC implementation in `dispatch/cron.rs` (ranges, lists, `*/n`, Sunday 0/7, day-or-weekday semantics).

## 4. Wiring

`apps/server/src/maintenance.rs` runs a dedicated dispatcher loop every two seconds for every store, plus log retention every ten minutes. The loop is independent of the ten-second workflow sweep so an event that is marked processed quickly is still visible in the one-hour fan-out window. The order matters: fan-out must see the outbox row before the workflow marks it processed; delivery rows are idempotent either way.

## 5. Admin API

| Method | Path                                                         | Permission      |
| ------ | ------------------------------------------------------------ | --------------- |
| GET    | `/api/v1/admin/plugins/{id}/deliveries`                      | `plugins.read`  |
| POST   | `/api/v1/admin/plugins/{id}/deliveries/{delivery_id}/replay` | `plugins.write` |
| GET    | `/api/v1/admin/plugins/{id}/jobs`                            | `plugins.read`  |
| POST   | `/api/v1/admin/plugins/{id}/jobs/{name}/run`                 | `plugins.write` |

A replay resets `attempts`, status and availability. A manual job run validates the name against the manifest first.

## 6. Verification

| Check                                                                | Result                                                                                          |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `dispatch::cron` unit tests                                          | 3 passed                                                                                        |
| `admin_plugins::event_subscriptions_fan_out_and_jobs_are_manageable` | subscriptions synced; outbox fanned out; cron enqueued; delivery replay and manual job accepted |
| `runtime_workerd::delivers_events_and_runs_jobs`                     | real workerd: delivery `done`, job `completed`                                                  |
| Full affected suites                                                 | passed                                                                                          |

## 7. Limitations

| Limitation                              | Impact                                                  | Owner        |
| --------------------------------------- | ------------------------------------------------------- | ------------ |
| No admin UI for deliveries/jobs         | API only                                                | P7           |
| No bulk replay or dead-letter purge     | Operators replay one at a time                          | P7           |
| Fixed retry policy                      | 5 attempts, exponential backoff                         | P7 if needed |
| Cron is UTC and five-field only         | No seconds or time zones                                | P7           |
| Event payloads are snapshots            | Matches the design; plugins fetch live data when needed | —            |
| Dispatcher runs on the maintenance tick | Delivery latency is up to one tick (10 s)               | P6 tuning    |
