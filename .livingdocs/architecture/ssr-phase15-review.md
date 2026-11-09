---
type: Architecture Review
title: Universal SSR Runtime Phase 15 Review
description: Generic Fetch Handler and framework-agnostic contract review.
kind: review
timestamp: 2026-11-09T16:00:00Z
---

# Universal SSR Runtime Phase 15 Review

## Scope

Phase 15 adds a framework-free artifact:

- `templates/storefront-generic` is a plain `export default { fetch }` Worker.
- It serves health, SSR HTML, headers/body echo, streaming, duplicate cookies,
  redirects and an intentional error.
- Its client asset is served under `/_fastcart/assets/`.
- `scripts/package.mjs` builds the standard FastCart artifact with the generic
  framework metadata.
- A workerd smoke test and CI job validate the artifact.

No framework-specific code was added to the Rust gateway or runtime manager.

## Acceptance checklist

| Criterion                                    | Status  | Evidence                                                                                |
| -------------------------------------------- | ------- | --------------------------------------------------------------------------------------- |
| A non-TanStack/non-Astro Fetch handler works | Pass    | Generic artifact smoke test                                                             |
| The core gateway remains framework-agnostic  | Pass    | No Rust changes required for this template                                              |
| Artifact layout is standard                  | Pass    | `fastcart-ssr.json`, `worker/index.js`, `client/`                                       |
| Health contract works                        | Pass    | `/__fastcart/health`                                                                    |
| Streaming/cookies/redirects/errors           | Pass    | Smoke test covers all                                                                   |
| Static assets                                | Partial | Asset is packaged and URL is emitted; Rust asset serving is shared with other templates |
| SvelteKit/React Router/Vike templates        | Not yet | Generic template is the first additional target                                         |
| Browser hydration                            | Not yet | Generic asset sets a DOM marker but no browser harness                                  |

## Findings

1. **Generic Worker is the clearest runtime contract test.** It avoids framework
   adapters and proves the Fetch handler/artifact contract directly.
2. **No runtime change was necessary.** This is the expected outcome and
   indicates the abstraction boundary is in the right place.
3. **Framework adapters still need maintained fixtures.** SvelteKit, React
   Router and Vike should reuse the same artifact packager and smoke harness.
4. **The generic smoke test duplicates the Astro/TanStack harness.** The three
   scripts should eventually share a single fixture runner.
5. **The generic asset is not content-hashed.** Production templates must emit
   hashed assets so the immutable cache policy remains valid.

## Verification

```bash
CI=true pnpm --filter @fastcart/storefront-generic build:artifact
CI=true pnpm --filter @fastcart/storefront-generic test:artifact
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 15.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README lists the generic template.

## Gate decision

Phase 16 (additional framework adapters or advanced isolation) may start. The
generic template establishes the contract for any future framework.
