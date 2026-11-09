---
type: Architecture Review
title: Universal SSR Runtime Phase 0 Review
description: Validation results, deviations and follow-ups for the workerd Phase 0 gate.
kind: review
timestamp: 2026-11-09T03:45:00Z
---

# Universal SSR Runtime Phase 0 Review

## Scope

Phase 0 of the [Universal SSR Runtime](universal-ssr-runtime.md) plan validates
that workerd can execute the Worker artifacts FastCart intends to accept, before
any production `fastcart-ssr` code is written.

Pinned runtime: workerd `v1.20261109.1`, Linux x86_64, SHA-256
`543f30be7dc9b6e263226fa08daa86cbb505b4b407048fbcdb82d0f1b910ad80`.
The binary is not committed; `scripts/install-workerd.sh` downloads and verifies
it, and `experiments/workerd` contains the fixtures.

## Acceptance checklist

| Criterion                                            | Status  | Evidence                                                                                                                                                                  |
| ---------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| workerd starts on the target platform                | Pass    | `workerd --version`; Phase 0 test starts four services on loopback ports                                                                                                  |
| A standard Fetch Handler executes                    | Pass    | `configs/basic.capnp` returns method, path and query                                                                                                                      |
| Required Web APIs are available                      | Pass    | `configs/web-api.capnp` checks Request/Response/Headers/URL, streams, encoding, Web Crypto, AbortController and timers                                                    |
| Streaming works incrementally                        | Pass    | `configs/streaming.capnp` delivers three chunks about 300 ms apart                                                                                                        |
| Service bindings work                                | Pass    | `configs/service-bindings.capnp` calls an internal service and receives store context                                                                                     |
| TanStack Start SSR works                             | Pass    | TanStack Start `1.169.5` output served an SSR document under workerd                                                                                                      |
| Astro SSR works                                      | Pass    | Astro `7.4.7` + `@astrojs/cloudflare` `14.4.6` output served an SSR document                                                                                              |
| Static assets can be served through the architecture | Partial | Client assets are emitted under `dist/client`; Rust owns their serving, so Phase 0 did not run a Rust asset server. Phase 4/7 must add the asset route and cache tests.   |
| Runtime configuration generation is feasible         | Pass    | Single-module and service-binding configs were generated and accepted by the pinned binary                                                                                |
| Cloudflare-specific bindings are documented          | Partial | `cloudflare:workers`, `nodejs_compat`, `ASSETS`, `IMAGES` and `SESSION` behavior is recorded; the final allowlist belongs to the artifact schema and validator in Phase 2 |

## Findings

1. **Framework output is not automatically a single file.** TanStack Start emits
   `dist/server/index.js` plus `assets/*.js`; Astro emits `entry.mjs` plus
   `chunks/*.mjs`. The artifact packager must either preserve the module graph
   and teach the workerd config generator to load every module, or bundle the
   entry. Phase 0 used esbuild as a validation shortcut.
2. **Node compatibility is date-driven.** At compatibility date `2026-11-01`
   workerd treats `nodejs_compat` as the default. Both framework outputs import
   `node:*` modules and ran without extra flags. The artifact manifest still
   records the compatibility date explicitly.
3. **`cloudflare:workers` is a runtime module, not a FastCart API.** TanStack
   uses it for bindings. FastCart must inject only the bindings it supports and
   reject artifacts that require unknown bindings.
4. **Framework adapters assume Cloudflare bindings.** Astro's adapter emitted a
   `wrangler.json` with `ASSETS`, `IMAGES` and `SESSION`; a plain SSR page ran
   without them, but production features may not. The artifact contract must
   declare required bindings.
5. **Workerd is a process boundary, not a sandbox.** Phase 0 did not test hostile
   code; ADR 0003 remains the trust model. Untrusted artifacts still need an OS
   or VM boundary.

## Deviations and follow-ups

- Framework fixtures are documented in `experiments/workerd/frameworks/README.md`
  rather than committed as installable apps. Phase 5 (TanStack Start) and Phase
  11 (Astro) replace the notes with maintained templates and CI fixtures.
- Static asset serving is deferred to the Rust asset route in Phase 4/7; Phase 0
  only confirms the framework emits a client asset directory.
- Outbound `fetch` was checked for availability and service-binding behavior,
  but not against an external host. The gateway and egress policy are Phase 3/4
  work.
- The artifact packager must choose between graph-preserving and bundled
  artifacts in Phase 2; the workerd config generator must support the chosen
  representation.

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 0
  as complete and the overall implementation as partial.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- `experiments/workerd/README.md` and its `frameworks/README.md` are the
  reproducible fixture notes.

## Gate decision

Phase 1 may start. The core Worker contract, streaming and both target
framework outputs are proven. Static asset delivery and committed framework
fixtures are tracked as Phase 2/4/5/11 deliverables, not as Phase 0 claims.
