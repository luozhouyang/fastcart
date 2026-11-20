# Plugin Runtime P0 Validation: Checklist and Script Design

**Status:** Executed 2026-11-10. Results are in [Plugin Runtime P0 Report](plugin-p0-report.md). This document remains the checklist and harness design.
**Scope:** prove or disprove the runtime assumptions in P0 before any product code is written.
**Duration:** ~3 person-days.
**Output:** a reproducible report, committed fixtures and scripts, and a go / no-go recommendation.

---

## 1. Purpose and Exit Criteria

P0 does not build the plugin system. It answers a fixed set of questions about the pinned workerd build and the HTML injector design, so that P1–P4 are not built on assumptions.

The final run executed 45 checks: 27 gates and 18 informational measurements. `scripts/plugin-p0.sh` reproduces it.

P0 exits when:

1. Every case in section 3 has a recorded result: pass, fail, or informational with measurements.
2. The measured numbers are written to `report.md` (cold start, request latency, RSS, compile time, injection overhead).
3. Every assumption marked **load-bearing** below is either confirmed or has a documented fallback with a cost estimate.
4. The decision log and risk table in the main design doc are updated with anything P0 changed.
5. A go / no-go recommendation is written, including the cost of each fallback.

P0 does not cover: plugin SDK ergonomics, admin UI rendering, storefront widget behavior, load testing at scale, or a security audit.

### Load-bearing assumptions

| ID  | Assumption                                                                                                                       | If false                                                                                                                |
| --- | -------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| A1  | The pinned workerd loads plugin code at runtime through the Worker Loader with `--experimental`.                                 | Fall back to static services with a process restart per activation; add 2–3 weeks to P1/P2 and accept the restart cost. |
| A2  | A loaded worker receives only the capabilities passed in its `env`, and `globalOutbound: null` blocks all global network access. | Stop. The capability boundary is not enforceable and the design must change.                                            |
| A3  | A plugin can be given a service binding through `env` and use it as its only outbound path.                                      | Use per-plugin bridges with generated configuration (restart required); revisit the runtime choice.                     |
| A4  | Module graphs up to realistic plugin sizes compile within an acceptable budget.                                                  | Cap plugin size, pre-compile, or revisit the loader.                                                                    |
| A5  | A streamed HTML response can be transformed safely with a bounded buffer.                                                        | Buffer the full response (worse latency and memory) or require theme markers only.                                      |
| A6  | `nodejs_compat` exposes the allowlisted modules and not `fs`/`net`.                                                              | Adjust the allowlist and SDK documentation.                                                                             |

---

## 2. Environment

| Item       | Requirement                                                                                                           |
| ---------- | --------------------------------------------------------------------------------------------------------------------- |
| OS         | Linux x86_64 (the pinned build is `workerd-linux-64`); record if another platform is used                             |
| workerd    | `experiments/workerd/bin/workerd`, version `2026-11-09`, SHA-256 from `scripts/install-workerd.sh`                    |
| Node       | ≥ 22 (global `fetch`, `WebSocket`, `node:test`)                                                                       |
| Flags      | `workerd serve --experimental <config>`; every result must state the flag state                                       |
| Network    | Loopback only. No test may reach the public internet; egress tests must assert failure                                |
| Ports      | Allocated per suite by binding `127.0.0.1:0`, reading the port, then closing; the port is substituted into the config |
| Artifacts  | `experiments/workerd/plugins/` (binary and generated configs are gitignored)                                          |
| Repetition | Timing cases run three times; report the median and the spread                                                        |

Environment facts are captured at the start of every run:

```text
workerd --version
uname -a
node --version
sha256sum experiments/workerd/bin/workerd
```

---

## 3. Test Inventory

Legend: **P** = pass/fail gate, **I** = informational measurement.

### A. Loader semantics

| ID  | Question                                                                                    | Fixture                         | Pass criterion                                                | Type |
| --- | ------------------------------------------------------------------------------------------- | ------------------------------- | ------------------------------------------------------------- | ---- |
| V01 | Does configuration load without `--experimental`?                                           | `config/loader.capnp`           | Load fails with the documented "experimental feature" error   | P    |
| V02 | Does it load with `--experimental`, and does `LOADER.get` + `getEntrypoint().fetch()` work? | `fixtures/hello`                | HTTP 200 with the fixture body                                | P    |
| V03 | Is the loaded worker cached per loader id?                                                  | `fixtures/hello`                | Module-level state survives between calls for the same id     | P    |
| V04 | Does a new digest produce a fresh instance?                                                 | `fixtures/hello@v1`, `hello@v2` | Different body for each id; v1 state unchanged after v2 loads | P    |
| V05 | What happens when `getCode` throws or the bundle is missing?                                | `fixtures/missing`              | Deterministic 502/500; workerd stays healthy                  | P    |

### B. Capabilities and egress

| ID  | Question                                                            | Fixture               | Pass criterion                                                                                    | Type |
| --- | ------------------------------------------------------------------- | --------------------- | ------------------------------------------------------------------------------------------------- | ---- |
| V06 | Does `env` deliver a service binding to the loaded worker?          | `fixtures/capability` | The plugin reaches the mock commerce through `env.API` and the mock records the request           | P    |
| V07 | Does the plugin see any binding it was not given?                   | `fixtures/capability` | `env` contains exactly the injected keys; no disk, no internet service, no other plugin's binding | P    |
| V08 | Does `globalOutbound: null` block `fetch()`?                        | `fixtures/egress`     | `fetch("https://example.com")` throws; no connection attempt is observed                          | P    |
| V09 | Does it also block `connect()`, WebSocket outbound, and `node:net`? | `fixtures/egress`     | All throw; no outbound socket is created                                                          | P    |

### C. Limits (currently expected to be unenforced)

| ID  | Question                                                        | Fixture                | Pass criterion                                       | Type |
| --- | --------------------------------------------------------------- | ---------------------- | ---------------------------------------------------- | ---- |
| V10 | Is `cpuMs` enforced for a synchronous loop?                     | `fixtures/loop-sync`   | Record: interrupted or completed, and after how long | I    |
| V11 | Is `cpuMs` enforced for an async loop with yields?              | `fixtures/loop-async`  | Record                                               | I    |
| V12 | Are entrypoint-level limits different from `WorkerCode.limits`? | `fixtures/loop-sync`   | Record both call forms                               | I    |
| V13 | Is `subRequests` enforced?                                      | `fixtures/subrequests` | Record how many requests succeed with a limit of 5   | I    |

> V10–V13 are informational for workerd behavior, but **A-gate for the design**: if unenforced (expected), host-side enforcement must be designed and approved before P2. The harness must always kill a runaway fixture after 10 seconds.

### D. Module graph and size

| ID  | Question                                                          | Fixture                            | Pass criterion                                                 | Type |
| --- | ----------------------------------------------------------------- | ---------------------------------- | -------------------------------------------------------------- | ---- |
| V14 | Do relative imports between modules work?                         | `fixtures/modules`                 | `./util.js` resolves; no bare specifiers                       | P    |
| V15 | Which module types are accepted (`text`, `json`, `data`, `wasm`)? | `fixtures/module-types`            | Documented per type; failures are explicit                     | I    |
| V16 | What is the effective module-size ceiling?                        | generated bundles at 8/32/64 MiB   | Record the largest accepted size and the failure mode above it | I    |
| V17 | How long does the first `LOADER.get` take for a realistic bundle? | `fixtures/realistic` (~1 MB graph) | Record compile time; flag above 500 ms                         | I    |

### E. Lifecycle, concurrency, and memory

| ID  | Question                                                           | Fixture                                 | Pass criterion                                                         | Type |
| --- | ------------------------------------------------------------------ | --------------------------------------- | ---------------------------------------------------------------------- | ---- |
| V18 | Are concurrent calls to the same loader id serialized or parallel? | `fixtures/concurrency`                  | Record behavior and wall time for 20 concurrent calls                  | P    |
| V19 | Do two loader ids run independently?                               | `fixtures/loop-sync` + `fixtures/hello` | The healthy plugin keeps answering while the other is blocked          | P    |
| V20 | Is there an observable `dispose()` on the stub?                    | `fixtures/hello`                        | Record presence and effect on the next `get()`                         | I    |
| V21 | Does repeated load/unload leak memory?                             | 50 iterations of `get` + drop           | RSS growth recorded; flag above 50 MB                                  | I    |
| V22 | What happens to in-flight requests when workerd is killed?         | `fixtures/slow`                         | Requests fail with a clean 502; the supervisor restarts; no host crash | P    |

### F. Protocol

| ID  | Question                                                                         | Fixture            | Pass criterion                                                                                                         | Type |
| --- | -------------------------------------------------------------------------------- | ------------------ | ---------------------------------------------------------------------------------------------------------------------- | ---- |
| V23 | Does a WebSocket upgrade reach a loaded worker?                                  | `fixtures/ws`      | 101; bidirectional messages work through the gateway                                                                   | P    |
| V24 | Does a streamed response preserve chunk order and timing?                        | `fixtures/stream`  | First chunk arrives before the last is produced; content matches                                                       | P    |
| V25 | Are multiple `Set-Cookie` headers preserved through the loader path?             | `fixtures/cookies` | Two cookies arrive                                                                                                     | P    |
| V26 | Are custom request headers visible to the plugin, and can it forge trusted ones? | `fixtures/headers` | Host-injected headers are visible; the plugin cannot alter the host's token (Rust validation is simulated by the mock) | P    |

### G. `nodejs_compat`

| ID  | Question                                                | Fixture                    | Pass criterion                                                                           | Type |
| --- | ------------------------------------------------------- | -------------------------- | ---------------------------------------------------------------------------------------- | ---- |
| V27 | Do the allowlisted modules import and behave?           | `fixtures/node-allow`      | `Buffer`, `process`, `events`, `path`, `util`, `crypto`, `stream` all work as documented | P    |
| V28 | Are `fs` / `net` / `tls` / `child_process` unavailable? | `fixtures/node-deny`       | Import throws or the API is absent, with a recorded error                                | P    |
| V29 | Does `process.env` exist, and what does it contain?     | `fixtures/node-allow`      | Only explicitly injected values; no host environment leakage                             | P    |
| V30 | Does the allowlist change with compatibility flags?     | same fixture, flags on/off | Record the difference                                                                    | I    |

### H. HTML injector (reference implementation in Node, ported to Rust in P4)

| ID  | Question                                                              | Fixture                             | Pass criterion                                                | Type |
| --- | --------------------------------------------------------------------- | ----------------------------------- | ------------------------------------------------------------- | ---- |
| V31 | Does a comment marker inject in place and disappear?                  | `html/marker-comment.html`          | Output has the tags at the marker position; no marker remains | P    |
| V32 | Does the script marker form work in JSX-like markup?                  | `html/marker-script.html`           | Same as V31                                                   | P    |
| V33 | Does fallback inject before the last `</body>` when no marker exists? | `html/no-marker.html`               | Tags appear before the final closing tag                      | P    |
| V34 | Is `</body>` inside an inline script string ignored?                  | `html/body-in-script.html`          | Injection goes to the real closing tag                        | P    |
| V35 | Do markers survive every two-chunk split?                             | `html/marker-comment.html`          | For every split index, output equals the single-pass output   | P    |
| V36 | Are non-HTML, non-200, compressed, and fragment responses skipped?    | `html/*.html` + header matrix       | No injection; response unchanged                              | P    |
| V37 | Are duplicate markers handled deterministically?                      | `html/duplicate-marker.html`        | First marker wins; warning recorded                           | P    |
| V38 | Is a second pass idempotent?                                          | output of V31                       | Second injection produces no change                           | P    |
| V39 | Is a strict-mode theme without a marker left untouched?               | `html/no-marker.html` + strict flag | No injection; warning                                         | P    |

### I. Performance

| ID  | Question                                        | Method                               | Pass criterion                              | Type |
| --- | ----------------------------------------------- | ------------------------------------ | ------------------------------------------- | ---- |
| V40 | Cold start to first successful request          | spawn → poll                         | Record (reference: ~71 ms locally)          | I    |
| V41 | Warm request latency through gateway and bridge | 500 sequential requests              | p50 < 2 ms, p95 < 5 ms                      | I    |
| V42 | RSS with 10 loaded plugins                      | 10 fixtures + one request each       | Record; reference 96.8 MB                   | I    |
| V43 | Injection overhead                              | 1 MB HTML, marker and fallback paths | < 2 ms of CPU per response, constant memory | I    |

---

## 4. Detailed Cases

### V08 / V09 — egress closure

Setup: gateway loads `fixtures/egress` with `globalOutbound: null`; the mock commerce records every connection attempt.

Steps:

1. `GET /p/egress/fetch` → expect a thrown error inside the plugin, returned as JSON `{ blocked: true }`.
2. `GET /p/egress/connect` → `connect()` (or `node:net`) must throw.
3. `GET /p/egress/ws` → an outbound WebSocket must throw.
4. `GET /p/egress/dns` → `node:dns` must be unavailable (ties to V28).

Failure signal: any outbound TCP connection observed by the mock or the OS, or a successful HTTP status.

### V18 / V19 — concurrency and cross-plugin isolation

Setup: two loader ids: `blocker` (runs a 3 s synchronous loop) and `healthy` (returns JSON).

Steps:

1. Start `blocker`.
2. After 100 ms, call `healthy` ten times.
3. Record healthy latency and any timeouts.
4. Repeat with `blocker` doing an async loop that awaits timers.

Expected: with a single-threaded gateway, `blocker` may delay `healthy`. Record the magnitude; this is the input to the "shared loader risk" decision.

### V23 — WebSocket through the loader

Setup: `fixtures/ws` returns a 101 with a `WebSocketPair`, echoes messages.

Steps:

1. `new WebSocket("ws://127.0.0.1:<port>/p/ws/room")` from Node.
2. Assert `101` behavior, send `"ping"`, expect `"echo:ping"`.
3. Close and assert the plugin's close handler runs.

### V24 — streaming

Setup: `fixtures/stream` enqueues `"a"`, waits 300 ms, enqueues `"b"`.

Steps:

1. `fetch` and read the body reader.
2. Assert the first chunk arrives before 300 ms and the second after.

### V31–V39 — injector corpus

The Node reference implementation lives in `harness/inject.mjs` and is exercised against the corpus in `html/`.

Corpus requirements:

- `marker-comment.html`: both markers, nested tags, inline CSS and JS.
- `marker-script.html`: both script markers, JSX-style attributes and self-closing tags.
- `no-marker.html`: a normal document with `</body>`.
- `no-closing-tag.html`: truncated document.
- `body-in-script.html`: `</body>` inside a template literal.
- `duplicate-marker.html`: two body markers.
- `fragment.html`: a partial without `<html>`.
- `compressed.html`: served with `content-encoding: gzip`.
- `1mb.html`: large document for V43.

Assertions:

- Byte equality against a `.expected.html` fixture, generated once and reviewed.
- For V35, every split index must reproduce the expected output.
- For V38, `inject(inject(x)) === inject(x)`.

### V40–V43 — measurements

Record raw samples in `report.json`; the report generator prints median, p95, and max. Cold start is measured from `spawn()` to the first 200 response, excluding port allocation.

---

## 5. Script Design

### 5.1 Directory layout

```text
experiments/workerd/plugins/
├── README.md
├── fixtures/
│   ├── hello/{index.js,plugin.json}
│   ├── capability/index.js
│   ├── egress/index.js
│   ├── loop-sync/index.js
│   ├── loop-async/index.js
│   ├── subrequests/index.js
│   ├── modules/{index.js,util.js}
│   ├── module-types/index.js
│   ├── concurrency/index.js
│   ├── slow/index.js
│   ├── ws/index.js
│   ├── stream/index.js
│   ├── cookies/index.js
│   ├── headers/index.js
│   ├── node-allow/index.js
│   ├── node-deny/index.js
│   └── missing/                     # no bundle on purpose
├── html/
│   ├── *.html
│   └── *.expected.html
├── harness/
│   ├── env.mjs                      # versions, paths, port allocation
│   ├── workerd.mjs                  # spawn, readiness, logs, stop
│   ├── config.mjs                   # Cap'n Proto generation
│   ├── commerce.mjs                 # mock platform API
│   ├── loader-gateway.mjs           # gateway source used by fixtures
│   ├── inject.mjs                   # reference HTML injector
│   ├── assert.mjs                   # assertion helpers + error messages
│   ├── report.mjs                   # report.json / report.md
│   └── fixtures.mjs                 # load fixture sources into module maps
├── suites/
│   ├── a-loader.test.mjs
│   ├── b-capability.test.mjs
│   ├── c-limits.test.mjs
│   ├── d-modules.test.mjs
│   ├── e-lifecycle.test.mjs
│   ├── f-protocol.test.mjs
│   ├── g-node.test.mjs
│   ├── h-injector.test.mjs
│   └── i-performance.test.mjs
└── report/                          # gitignored output
    ├── report.json
    └── report.md
```

### 5.2 Harness contracts

`harness/env.mjs`

```js
export const WORKERD =
  process.env.FASTCART_WORKERD_BIN ?? new URL("../../bin/workerd", import.meta.url).pathname;
export const EXPERIMENTAL = process.env.FASTCART_WORKERD_EXPERIMENTAL !== "0";

export async function freePort() {
  const server = net.createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}
```

`harness/workerd.mjs`

```js
export async function startWorkerd({ config, port, timeoutMs = 10_000 }) {
  const child = spawn(WORKERD, ["serve", ...(EXPERIMENTAL ? ["--experimental"] : []), config], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  const logs = collectLogs(child);
  await waitFor(`http://127.0.0.1:${port}/__fastcart/health`, timeoutMs);
  return { child, logs, stop: () => terminate(child) };
}
```

`harness/config.mjs`

```js
export function buildConfig({ port, commercePort, loaderId = "plugins" }) {
  return `using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "gateway", worker = .gateway),
    (name = "bridge", worker = .bridge),
    (name = "commerce", external = (address = "127.0.0.1:${commercePort}", http = ())),
  ],
  sockets = [(name = "http", address = "127.0.0.1:${port}", http = (), service = "gateway")],
);
const gateway :Workerd.Worker = (
  modules = [(name = "gateway.js", esModule = embed "gateway.js")],
  compatibilityDate = "2026-11-01",
  bindings = [
    (name = "LOADER", workerLoader = (id = "${loaderId}")),
    (name = "BRIDGE", service = "bridge"),
    (name = "COMMERCE", service = "commerce"),
  ],
);
const bridge :Workerd.Worker = (
  modules = [(name = "bridge.js", esModule = embed "bridge.js")],
  compatibilityDate = "2026-11-01",
  bindings = [(name = "UPSTREAM", service = "commerce")],
);
`;
}
```

`harness/commerce.mjs` records `{ method, url, headers, body }` and returns deterministic JSON. It refuses to bind to anything other than `127.0.0.1`.

`harness/loader-gateway.mjs` is the same gateway shape the product will use: route `/p/{id}@{digest}/{rest}`, load module maps from `harness/fixtures.mjs`, inject `env`, set `globalOutbound: null`, forward the request.

`harness/assert.mjs` provides `assertStatus`, `assertJson`, `assertThrows`, `assertWithin(ms)`, and `record(id, value)` for informational results.

### 5.3 Suites

Each suite is a `node:test` file with `before`/`after` hooks:

```js
import { test, before, after } from "node:test";
import assert from "node:assert/strict";

let runtime;
before(async () => {
  runtime = await startSuite("a-loader");
});
after(async () => {
  await runtime?.stop();
});

test("V02 loader loads and serves", async () => {
  const res = await fetch(`${runtime.base}/p/hello@v1/`);
  assert.equal(res.status, 200);
  assert.equal(await res.text(), "hello");
});
```

Execution:

```bash
node --test --test-concurrency=1 experiments/workerd/plugins/suites
```

`--test-concurrency=1` keeps port allocation and process counts deterministic. Suites that are pure (injector) can run in parallel later.

### 5.4 Reporting

`harness/report.mjs` writes `report/report.json`:

```json
{
  "environment": { "workerd": "2026-11-09", "sha256": "…", "node": "v22…", "experimental": true },
  "gates": { "V01": "pass", "V02": "pass", "V10": "informational" },
  "measurements": {
    "cold_start_ms": { "samples": [71, 69, 73], "median": 71 },
    "warm_latency_ms": { "p50": 0.43, "p95": 0.55, "p99": 1.29 },
    "rss_10_plugins_mb": 96.8,
    "compile_ms_1mb": 41
  },
  "notes": ["cpuMs not enforced; host-side enforcement required"]
}
```

`report.md` is generated from the same data and committed with the run date.

### 5.5 Fixture contract

Every fixture exports the plugin shape the product will use:

```js
export default {
  async fetch(request, env) { … },
  async admin(ctx, method, params) { … },   // optional
  async events(ctx, event) { … },           // optional
  async jobs(ctx, name, payload) { … },     // optional
};
```

The loader only calls `fetch` in P0. The other exports are exercised by contract tests later, not in P0.

### 5.6 Cleanup and isolation

- Every suite starts its own workerd on its own port and kills it in `after`, including on assertion failure (`process.on("exit")` guard).
- Generated configs live in a per-suite temp directory under `report/tmp/`, removed at the end.
- No test writes outside `experiments/workerd/plugins/report/`.
- Runaway fixtures are killed after 10 s by the harness; a timeout is reported as a failure of the harness, not of the fixture.
- The harness never sets `FASTCART_*` secrets and never binds a non-loopback address.

---

### 5.7 Wrapper script

`scripts/plugin-p0.sh` is the single entry point for a run:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLUGINS="$ROOT/experiments/workerd/plugins"
WORKERD="$ROOT/experiments/workerd/bin/workerd"

if [[ ! -x "$WORKERD" ]]; then
  echo "workerd not installed; run scripts/install-workerd.sh first" >&2
  exit 2
fi

export FASTCART_WORKERD_BIN="$WORKERD"
export FASTCART_WORKERD_EXPERIMENTAL="${FASTCART_WORKERD_EXPERIMENTAL:-1}"

node --test --test-concurrency=1 "$PLUGINS/suites"
node "$PLUGINS/harness/report.mjs" --write

echo "report: $PLUGINS/report/report.md"
```

Rules:

- Exit code 2 means the environment is missing; exit code 1 means a gate failed.
- Informational cases never fail the run; they are printed and recorded.
- The script never downloads workerd and never reaches the network.

---

## 6. Execution Plan

| Step                                                         | Cases   | Time |
| ------------------------------------------------------------ | ------- | ---- |
| 1. Harness skeleton (env, workerd, config, commerce, report) | —       | 0.5d |
| 2. Loader, capabilities, egress                              | V01–V09 | 0.5d |
| 3. Limits, modules, lifecycle                                | V10–V22 | 0.5d |
| 4. Protocol, node compat                                     | V23–V30 | 0.5d |
| 5. Injector corpus and reference implementation              | V31–V39 | 0.5d |
| 6. Measurements, report, recommendation                      | V40–V43 | 0.5d |

### Stop conditions

- **A1 false** (no loader): stop and rewrite P1/P2 around static services; add 2–3 weeks and accept restarts.
- **A2 false** (egress not closable, or ambient capabilities visible): stop. Do not proceed until the capability boundary is proven.
- **A4 false** (compile time unacceptable): cap plugin size and pre-warm on install; if still unacceptable, reconsider the substrate.
- **A5 false** (injector cannot be made safe): require markers only for supported themes and document that third-party themes get no storefront extensions.
- **V10–V13 unenforced** (expected): host-side enforcement design becomes a P2 prerequisite, not an optional hardening item.

---

## 7. Deliverables

1. `experiments/workerd/plugins/` with fixtures, harness, and suites (binary and reports gitignored).
2. `report/report.md` with environment, gate results, measurements, and notes.
3. `scripts/plugin-p0.sh` wrapper that runs the suites and fails on any gate failure.
4. Updates to the main design doc: decision log (if any assumption changed), 4.4 limits, 11 threat table, and the P0 completion note.
5. A written recommendation: proceed to P1, proceed with conditions, or change substrate.

---

## 8. Traceability

| P0 case | Design section | Risk                                   |
| ------- | -------------- | -------------------------------------- |
| V01–V05 | 4.1, 4.3       | Loader availability and lifecycle      |
| V06–V09 | 3.1, 9.2, 9.4  | T3, T4, T8                             |
| V10–V13 | 4.4, T5        | No hard CPU limit; shared process      |
| V14–V17 | 4.1, 5.4       | Module graph and build contract        |
| V18–V22 | 4.3, 4.4, T5   | Concurrency, isolation, crash handling |
| V23–V26 | 4.2.2, 8.3     | WebSocket, streaming, headers          |
| V27–V30 | 5.4, T10       | `nodejs_compat` surface                |
| V31–V39 | 8.1            | Marker protocol, fallback, streaming   |
| V40–V43 | 13             | Performance budgets and capacity       |

---

## 9. Open Items to Resolve During P0

1. Whether a per-plugin admission timeout in the gateway is enough to protect the shared loader, or whether the loader needs a watchdog process.
2. The exact `nodejs_compat` allowlist in this build, and whether it changes with the compatibility date.
3. The effective module-size ceiling and the compile-time curve; whether to pre-warm on install.
4. Whether `dispose()` exists and is safe to call while requests are in flight.
5. Whether the marker scanner should be a byte scanner or a small HTML tokenizer; the corpus will decide.
6. Whether the gateway should send `Accept-Encoding: identity` for all HTML or only when storefront extensions exist.

---

## Appendix A: Run Sheet

Copy this table into the report and fill it in during the run.

| ID  | Case                                            | Result | Evidence |
| --- | ----------------------------------------------- | ------ | -------- |
| V01 | Loader config rejected without `--experimental` | ☐      |          |
| V02 | Loaded worker serves a request                  | ☐      |          |
| V03 | Loader id caches the worker                     | ☐      |          |
| V04 | New digest loads a fresh instance               | ☐      |          |
| V05 | Missing bundle fails cleanly                    | ☐      |          |
| V06 | `env` service binding reaches the host          | ☐      |          |
| V07 | No ambient bindings are visible                 | ☐      |          |
| V08 | Global `fetch` blocked                          | ☐      |          |
| V09 | `connect` / WS / `node:net` blocked             | ☐      |          |
| V10 | `cpuMs` on a synchronous loop                   | ☐      |          |
| V11 | `cpuMs` on an async loop                        | ☐      |          |
| V12 | Entrypoint-level limits                         | ☐      |          |
| V13 | `subRequests` limit                             | ☐      |          |
| V14 | Relative imports resolve                        | ☐      |          |
| V15 | Module types accepted                           | ☐      |          |
| V16 | Module-size ceiling                             | ☐      |          |
| V17 | First-compile latency                           | ☐      |          |
| V18 | Concurrent calls to one id                      | ☐      |          |
| V19 | Cross-plugin isolation                          | ☐      |          |
| V20 | `dispose()` presence                            | ☐      |          |
| V21 | Repeated load/unload memory                     | ☐      |          |
| V22 | In-flight requests on kill                      | ☐      |          |
| V23 | WebSocket through the loader                    | ☐      |          |
| V24 | Streaming response                              | ☐      |          |
| V25 | Multiple `Set-Cookie` headers                   | ☐      |          |
| V26 | Header visibility and spoofing                  | ☐      |          |
| V27 | `nodejs_compat` allowlist                       | ☐      |          |
| V28 | `nodejs_compat` denylist                        | ☐      |          |
| V29 | `process.env` contents                          | ☐      |          |
| V30 | Flag on/off differences                         | ☐      |          |
| V31 | Comment marker injection                        | ☐      |          |
| V32 | Script marker injection                         | ☐      |          |
| V33 | Fallback before `</body>`                       | ☐      |          |
| V34 | `</body>` inside a JS string                    | ☐      |          |
| V35 | Every two-chunk split                           | ☐      |          |
| V36 | Skip matrix (type/status/encoding/fragment)     | ☐      |          |
| V37 | Duplicate markers                               | ☐      |          |
| V38 | Idempotent second pass                          | ☐      |          |
| V39 | Strict mode without a marker                    | ☐      |          |
| V40 | Cold start                                      | ☐      |          |
| V41 | Warm latency                                    | ☐      |          |
| V42 | RSS with 10 plugins                             | ☐      |          |
| V43 | Injection overhead                              | ☐      |          |
