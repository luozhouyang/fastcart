---
type: ADR
title: Checkout keeps hand-rolled compensation; the workflow engine is not yet used
description: The persistent workflow engine exists and is tested but no production workflow runs on it, so the money path does not get its leases, retries or compensation.
owner: luozhouyang
status: Accepted
implementation: partial
timestamp: 2026-11-20T00:00:00Z
---

# 0010. Checkout keeps hand-rolled compensation; the workflow engine is not yet used

## Context

`crates/workflows/src/engine.rs` is a generic persistent workflow engine:
executions, step checkpoints, retry counts and leases live in the store
database, and it deliberately never holds a SQLite transaction across an awaited
step. It is exercised by `crates/workflows/tests/engine.rs`.

Nothing in the product runs on it. `WorkflowEngine` is referenced only by its
own tests and the crate's re-export. The checkout workflow
(`crates/workflows/src/checkout.rs`) instead hand-rolls its own fencing and
compensation: a placement claim with a lease, `claim_placement` /
`release_placement` / `holds_placement`, and a `compensate` method that restocks,
cancels, refunds and now releases promotion claims.

So the README's "workflows compose them" describes an intent, not the money
path: checkout does not get the engine's checkpointing or its bounded retry of
failed compensation steps.

## Decision

Keep the engine, and do not migrate checkout onto it yet.

Migrating the money path means re-expressing stock reservation, promotion
claims, capture, order placement and compensation as engine steps, and
reconciling the engine's lease and retry semantics with the placement claim that
checkout already relies on. That is a project, not a fix, and it would need the
same care the current hand-rolled path has received.

What is decided here is that the current state is a known, recorded gap rather
than an implied guarantee:

- Checkout's own guarantees are the ones documented in `checkout.rs`: the
  placement claim fences concurrent completions, compensation is guarded by
  `holds_placement`, and each side effect (capture, order insert, usage claim,
  refund) carries its own idempotency key.
- The README must not claim the engine composes the checkout path.

`crates/commerce` was removed in the same pass: it was an empty Phase-0 shell
whose twelve modules were declared and never written, referenced by no manifest,
and it produced an unused-workspace-dependency warning on every build.

## Consequences

- The engine keeps its tests and stays available for the step-2 migration.
- Anyone reading the README should not infer that checkout is engine-backed.
- Migrating checkout to the engine remains open, and should be scheduled as
  work in its own right.
