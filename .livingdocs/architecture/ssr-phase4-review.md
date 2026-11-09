---
type: Architecture Review
title: Universal SSR Runtime Phase 4 Review
description: SSR gateway, trusted context and server wiring review.
kind: review
timestamp: 2026-11-09T05:30:00Z
---

# Universal SSR Runtime Phase 4 Review

## Scope

Phase 4 connects the runtime manager to the HTTP server:

- `fastcart_ssr::SsrGateway` now accepts a streaming `axum::body::Body`,
  strips client-supplied `x-fastcart-*`/forwarding headers, injects trusted
  store/deployment/request context and preserves a streamed response.
- `fastcart_api::UniversalStorefront` is the process-wide boundary; the
  storefront fallback forwards to it after store resolution, redirects,
  robots.txt and sitemap.xml.
- `apps/server/src/universal.rs` imports the configured artifact, starts the
  managed group, implements the trait and maps runtime failures to HTTP errors.
- `[ssr]` configuration and `FASTCART_SSR_*` environment overrides exist.

## Acceptance checklist

| Criterion                                             | Status  | Evidence                                                                                               |
| ----------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------ |
| Storefront requests reach the correct worker          | Partial | API test records trusted store context; `UniversalSsr` test starts workerd and renders                 |
| Internal headers cannot be spoofed                    | Pass    | Gateway unit test and real workerd test overwrite `x-fastcart-store-id` and `x-forwarded-*`            |
| Response headers are preserved                        | Pass    | Status/header test and multiple `Set-Cookie` test                                                      |
| Streaming HTML works                                  | Partial | Gateway and Axum body stream paths are implemented; the browser-level test waits for Phase 5 templates |
| Worker errors do not crash the server                 | Partial | Errors map to controlled responses; no fault-injection test yet                                        |
| Runtime unavailability produces a controlled response | Partial | `SsrError` mapping returns 502/503; no test forces a dead runtime through the HTTP server              |
| Timeouts exist                                        | Partial | One reqwest client timeout; separate connection/header/idle timeouts are Phase 9                       |
| Trusted context is typed and injected                 | Pass    | `SsrRequestContext` with store, deployment, request, locale, currency, scheme                          |
| Store resolution remains Rust-owned                   | Pass    | Store is resolved before the gateway; Worker receives headers only                                     |
| Legacy and universal runtimes cannot both be enabled  | Pass    | Config validation rejects both                                                                         |

## Findings

1. **The browser path is not yet exercised end-to-end.** The API test uses a
   recording `UniversalStorefront`, and `apps/server` has a real workerd test;
   the two are not joined through a running `fastcart-server` with a provisioned
   store. Phase 5's TanStack Start fixture closes this with a full request test.
2. **Static assets still use the legacy theme-asset route.** The gateway does
   not yet serve `/_fastcart/assets/`; that is Phase 7.
3. **Robots and sitemap stay in Rust.** The fallback handles those before the
   universal branch, which matches the plan's static/discovery ownership.
4. **One manager and one group per process.** Multi-store deployment selection
   is Phase 8; this phase always injects the configured deployment id.
5. **Internal token is optional.** Loopback is enforced, so missing tokens are
   acceptable for local execution but should be mandatory for any later
   non-loopback topology.
6. **No crash-triggered restart.** A manager restart is available in the API,
   but the HTTP path does not yet detect and replace a dead child automatically.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-ssr
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo clippy -p fastcart-ssr -p fastcart-api -p fastcart-server --all-targets -- -D warnings
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 4.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and `config.example.toml` document `[ssr]` and `FASTCART_SSR_*`.

## Gate decision

Phase 5 (TanStack Start integration) may start. The gateway contract, trusted
context and server boundary are stable; Phase 5 replaces the Phase 3 fixture
with a real framework worker and exercises the browser path end-to-end.
