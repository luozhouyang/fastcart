# Plugin Runtime P0 Report

**Date:** 2026-11-10
**Status:** Complete. Gate results are green; four design changes are required before P1.
**Companion documents:** [Plugin System (workerd)](workerd-plugin-system.md), [P0 Validation Checklist](plugin-p0-validation.md).

---

## 1. Verdict

**Proceed to P1 with conditions.** The Worker Loader, the capability boundary, the HTML injector, WebSockets, streaming, and multi-module graphs all work as designed. The one assumption that failed is the one already suspected: **loader limits are not enforced**, and a synchronous loop in one plugin blocks every other plugin in the shared process (2.9 s measured). Host-side enforcement and a process-restart watchdog are therefore prerequisites for P2, not hardening work.

| #   | Condition                                                                                                                                                              | Owner phase      |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| C1  | Host-side per-plugin request budget, concurrency semaphore, and subrequest counter; a watchdog that restarts the whole plugin host when a request exceeds its deadline | P2               |
| C2  | Document that all plugins share one process, so one runaway plugin restarts the host                                                                                   | P2 / operations  |
| C3  | Treat `nodejs_compat` as always-on; document the virtual filesystem and the disabled process-spawning paths                                                            | P7 docs / P2 SDK |
| C4  | Add the injection sentinel and make the head fallback immediate                                                                                                        | P4               |

---

## 2. Environment

| Item        | Value                                                                                        |
| ----------- | -------------------------------------------------------------------------------------------- |
| OS          | Linux x86_64                                                                                 |
| workerd     | `workerd 2026-11-09` (pinned `v1.20261109.1`, repository binary)                             |
| Node        | `v26.13.0` (global `fetch`, `WebSocket`, `node:test`)                                        |
| Flags       | `workerd serve --experimental`                                                               |
| Network     | Loopback only                                                                                |
| Harness     | `experiments/workerd/plugins/`                                                               |
| Raw results | `experiments/workerd/plugins/report/results.jsonl`, `summary.json`, `report.md` (gitignored) |

Reproduce with:

```bash
./scripts/plugin-p0.sh
```

The run takes about 32 seconds and starts a fresh workerd process per suite.

---

## 3. Results

**45 checks executed: 27 gates passed, 0 failed, 18 informational.** Every gate from the checklist is green; the informational cases are the load-bearing measurements.

### 3.1 Measurements

| Metric                              | Result                                       | Notes                                                  |
| ----------------------------------- | -------------------------------------------- | ------------------------------------------------------ |
| Cold start (spawn → first response) | **61.8 ms**                                  | Suite boot, includes config write                      |
| Warm request latency, 500 requests  | **p50 0.31–0.45 ms / p95 0.50–0.59 ms**      | Gateway → loader → plugin → bridge → mock host         |
| RSS with 10 loaded plugins          | **+17.5–17.6 MB** (75.9–76.4 → 93.5–94.0 MB) | ≈1.75 MB per plugin                                    |
| RSS with 30 loaded plugins          | **+59.5 MB** (75.9 → 135.4 MB)               | ≈1.98 MB per plugin                                    |
| 1 MB single-module compile          | **19–28 ms**                                 | 41,185 top-level statements                            |
| 8 MB graph over 8 modules           | **120–178 ms**                               | Accepted                                               |
| 8 MB in a single module             | **502**                                      | `RangeError: Maximum call stack size exceeded` at load |
| Injector overhead                   | **1.35–1.61 ms per 1 MB document**           | 50 iterations                                          |
| Concurrent calls to one loader id   | **20 in flight, 110 ms**                     | Requests interleave at `await` points                  |
| Healthy plugin while another spins  | **2,909 ms**                                 | Blocked by the spinning plugin in the same process     |

### 3.2 Gate results by area

| Area                    | Cases    | Result                                                                                                                                           |
| ----------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| Loader semantics        | V01–V05  | Pass. Config rejected without `--experimental`; load, cache, and digest isolation work; missing bundle returns 502 and the runtime stays healthy |
| Capabilities and egress | V06–V09  | Pass. `env` carries exactly the two injected keys; bridge strips cookies and client store headers; global `fetch` and outbound WebSocket throw   |
| Module graph            | V14–V16b | Pass/informational. Relative imports, JSON and text modules work; module shape matters more than total size                                      |
| Lifecycle               | V18–V22  | Pass/informational. Concurrency interleaves; memory grows ~2 MB per plugin; a kill fails in-flight requests cleanly                              |
| Protocol                | V23–V26  | Pass. WebSocket 101, streaming with correct chunk timing, multiple `Set-Cookie`, host header override                                            |
| `nodejs_compat`         | V27–V30  | Pass with caveats. Built-ins are always available; `fs` is virtual; `net` is blocked; `child_process` methods are unimplemented                  |
| Injector                | V31–V39  | Pass. Markers, fallback, split-point equivalence (297 splits), duplicate handling, idempotency, strict mode                                      |
| Performance             | V40–V43  | Informational; see 3.1                                                                                                                           |

---

## 4. Findings

### F1. The Worker Loader is usable in the pinned build (confirmed)

With `--experimental`, `LOADER.get(id, getCode)` loads a module graph, caches it per id, and serves through `getEntrypoint().fetch()`. Module-level state persists across requests for the same id; a new digest produces a new instance without disturbing the old one. Without the flag, configuration loading fails with an explicit error.

**Action:** none. The design in 4.1 stands.

### F2. The capability boundary holds (confirmed)

The loaded worker's `env` contains exactly `FASTCART_API` and `FASTCART_PLUGIN_ID`. `globalOutbound: null` blocks `fetch`, outbound `WebSocket`, and `net.connect` with a clear runtime error. The bridge strips `cookie` and client-supplied `x-fastcart-store-id`, and the gateway overrides spoofed store headers. The only way out is the injected bridge.

**Action:** none for P1/P2. Keep the "one bridge, one plugin id" shape.

### F3. Loader `limits` are not enforced (confirmed)

`cpuMs: 100` did not interrupt a 300 ms synchronous loop (response 200, 308 ms). The same held for an async loop and for entrypoint-level limits (`cpuMs: 50`). `subRequests: 5` did not stop eight consecutive subrequests (all 200).

**Action (C1):** implement host-side enforcement in P2:

- per-request deadline in the Rust gateway (already planned) plus a _watchdog_ that restarts the plugin host process when a request exceeds the deadline;
- a per-plugin semaphore and queue;
- a subrequest budget in the bridge, counting calls and rejecting above the limit;
- document that a synchronous infinite loop can only be stopped by restarting the process.

### F4. A runaway plugin blocks every other plugin (confirmed, severe)

While one plugin ran a 3 s synchronous loop, a second, healthy plugin took **2,909 ms** to answer. They share one workerd process. This is the strongest argument for the deferred per-plugin process isolation, and it must be visible in operator documentation.

**Action (C2):** state in the admin plugin detail and operator docs that plugins share a runtime process. Add a `plugin runtime restarted` audit event and metric. Keep the deferred `isolation=process` item; it is now justified by measurement rather than speculation.

### F5. Concurrent calls interleave (confirmed)

Twenty concurrent requests to the same loader id all ran concurrently (`maxActive = 20`, 110 ms total for 20 × 100 ms waits). The isolate is single-threaded but yields at `await`, so plugins must be concurrency-safe.

**Action:** document in the SDK guide; keep the default per-plugin concurrency limit at 8; add a concurrency test to the plugin template.

### F6. Memory per plugin is about 1.75–2 MB (measured)

Ten plugins added 17.5–17.6 MB (1.75–1.76 MB each); thirty plugins added 59.5 MB (1.98 MB each). This is higher than the 1.1 MB estimate from static services, because each loaded worker carries its own isolate.

**Action:** update capacity planning to ~2 MB per loaded plugin; the plugin host baseline remains ~75 MB.

### F7. Module shape matters more than total size (confirmed)

A 1 MB single module compiles in 19–28 ms. An 8 MB graph across eight modules loads in 120–178 ms. The same 8 MB in one module fails at load with a V8 "Maximum call stack size exceeded" — not a size limit, but a parser/shape limit from ~500k top-level statements.

**Action:** in the build contract (5.4), require authors to bundle into multiple modules (which normal bundlers do), and cap a _single_ module at a conservative size (for example 4 MB) while allowing the whole graph up to the documented limit. Add a preflight warning for single modules above the cap.

### F8. Node built-ins are always on, and `fs` is virtual (confirmed, doc change)

Without the `nodejs_compat` flag, `Buffer`, `process`, `path`, `util`, `crypto`, `stream`, and `events` all worked (V30). The flag is not a capability gate in this build.

Observed surfaces:

| Module                | Behavior                                                                                                                                                                        |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `node:fs`             | Importable. Virtual root contains `bundle`, `tmp`, `dev`; reading `/etc/hostname` fails with ENOENT; writes to `/tmp` succeed inside the virtual FS. No host filesystem access. |
| `node:net`            | Importable; `connect` is rejected by the outbound policy.                                                                                                                       |
| `node:tls`            | Importable; no connection attempted in P0.                                                                                                                                      |
| `node:dns`            | Importable; lookups not exercised.                                                                                                                                              |
| `node:child_process`  | Importable; `execSync` and `spawnSync` throw `ERR_METHOD_NOT_IMPLEMENTED`.                                                                                                      |
| `node:worker_threads` | Importable; worker creation not exercised.                                                                                                                                      |
| `process.env`         | Contains text bindings from `env` (for example `FASTCART_PLUGIN_ID`); never place secrets in text bindings.                                                                     |

**Action (C3):** rewrite 5.4 as observed behavior, not policy. Add "never put secrets in text bindings" to the SDK/ops guide. Add `worker_threads` and `dns` to the P1 hardening list.

### F9. The injector design works; two refinements are needed (confirmed)

All marker and fallback cases pass, including 297 two-chunk split points and idempotency. Two refinements came out of the implementation:

1. **Sentinel.** Idempotency requires a marker in the injected output. The reference implementation emits `<!--fastcart:extensions-injected-->` once per response; a second pass detects it and returns the input unchanged. Without it, the fallback injects twice.
2. **Head fallback is immediate.** Waiting for EOF to find `</head>` would push the head far outside any tail buffer on large pages. The reference injects head tags as soon as `</head>` is seen, and only the body fallback uses the 32 KB tail to find the last `</body>`.

**Action (C4):** update 8.1 and 8.2 with the sentinel and the head-immediate rule; port the reference implementation to Rust in P4.

### F10. Killing workerd fails in-flight requests cleanly (confirmed)

A SIGKILL during an in-flight request produced `TypeError: fetch failed` at the caller and no host crash. The supervisor and restart policy are P2 work; the failure mode is acceptable.

---

## 5. Deviations from the Checklist

| Item | Deviation                                                            | Reason                                                                                |
| ---- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| V16  | Split into V16 (8 MB across 8 modules) and V16b (8 MB in one module) | The first fixture was not actually large; splitting exposed the shape dependency      |
| V20  | Recorded as "not exposed"                                            | The gateway does not retain stubs; `dispose` needs a debug path in P1                 |
| V23  | WebSocket tested with the Node global client                         | No extra dependency required                                                          |
| V28  | Assertions changed from "cannot import" to "cannot use"              | Node built-ins import but capabilities are blocked; that is the meaningful property   |
| V30  | Module availability without the flag                                 | Not in the original plan; added after the first run showed built-ins always available |

---

## 6. What This Changes in the Design

| Document section     | Change                                                                                                                      |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| 4.4 limits           | Replace "may be unenforced" with measured results; add the watchdog and subrequest counter                                  |
| 5.4 build contract   | Replace the `nodejs_compat` allowlist with observed behavior; add the single-module cap and the text-binding secret warning |
| 8.1 injection        | Add the sentinel; head fallback is immediate; body fallback uses the 32 KB tail                                             |
| 8.2 injected content | Include the sentinel comment                                                                                                |
| 13 performance       | Cold start 62 ms; warm p50 0.31–0.45 ms; ~2 MB per plugin; 1.35–1.61 ms injection per MB                                    |
| 17 risks             | Upgrade T5 and T10 with measured evidence; add the shared-process contention risk                                           |
| Appendix A.3         | Record that the loader exists but limits do not work, and the 2.9 s contention measurement                                  |

---

## 7. Next Steps

1. Apply the four conditions (C1–C4) to the design document.
2. Start P1: manifest schema, artifact store, code delivery endpoint, admin API.
3. Before P2, implement and test the watchdog: a request that exceeds its deadline triggers a host restart, with a metric and audit record.
4. Before P4, port the reference injector to Rust and reuse the corpus as unit tests.
5. Add `worker_threads`, `dns`, and `tls` behavior to the P1 hardening list.

---

## Appendix: Case Index

| ID      | Case                             | Status                          |
| ------- | -------------------------------- | ------------------------------- |
| V01     | Loader requires `--experimental` | pass                            |
| V02     | Load and serve                   | pass                            |
| V03     | Loader id caches                 | pass                            |
| V04     | New digest, fresh instance       | pass                            |
| V05     | Missing bundle fails cleanly     | pass                            |
| V06     | `env` service binding            | pass                            |
| V07     | No ambient bindings              | pass                            |
| V08     | Global `fetch` blocked           | pass                            |
| V09     | Outbound WebSocket blocked       | pass                            |
| V10     | `cpuMs` sync loop                | informational: not enforced     |
| V11     | `cpuMs` async loop               | informational: not enforced     |
| V12     | Entrypoint limits                | informational: not enforced     |
| V13     | `subRequests`                    | informational: not enforced     |
| V14     | Relative imports                 | pass                            |
| V15     | JSON and text modules            | informational: accepted         |
| V16     | 8 MB graph                       | informational: 120–178 ms       |
| V16b    | 8 MB single module               | informational: 502              |
| V17     | 1 MB compile                     | informational: 19–28 ms         |
| V18     | Concurrency                      | pass: interleaved               |
| V19     | Cross-plugin isolation           | informational: 2.9 s block      |
| V20     | `dispose`                        | informational: not exposed      |
| V21     | 30 loads memory                  | informational: +59.5 MB         |
| V22     | Kill in flight                   | informational: clean failure    |
| V23     | WebSocket                        | pass                            |
| V24     | Streaming                        | pass                            |
| V25     | `Set-Cookie`                     | pass                            |
| V26     | Header spoofing                  | pass                            |
| V27     | Node allowlist                   | pass                            |
| V28     | Node denylist                    | pass                            |
| V29     | `process.env`                    | pass                            |
| V30     | Modules without the flag         | informational: available        |
| V31–V39 | Injector                         | pass                            |
| V40     | Cold start                       | informational: 62 ms            |
| V41     | Warm latency                     | informational: p50 0.31–0.45 ms |
| V42     | 10-plugin RSS                    | informational: +17.5–17.6 MB    |
| V43     | Injection overhead               | informational: 1.6 ms/MB        |
