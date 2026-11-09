---
type: Architecture Review
title: Universal SSR Runtime Phase 12 Review
description: Advanced asset caching and preview review.
kind: review
timestamp: 2026-11-09T13:00:00Z
---

# Universal SSR Runtime Phase 12 Review

## Scope

Phase 12 adds cache behavior at the universal boundary:

- Static assets expose an ETag derived from size and modification time, support
  `If-None-Match` with `304`, and accept one byte range with `206`/`416`.
- Asset responses advertise `Accept-Ranges: bytes`.
- Artifact assets keep the immutable one-year cache policy.
- SSR documents default to `Cache-Control: no-store` unless the Worker sets a
  cache header explicitly.
- Retired asset roots remain readable during replacement draining.

Preview remains on the existing legacy `/preview` route; the universal runtime
does not yet accept a signed preview context.

## Acceptance checklist

| Criterion                             | Status  | Evidence                                                                     |
| ------------------------------------- | ------- | ---------------------------------------------------------------------------- |
| Hashed assets are cached immutably    | Pass    | `Cache-Control: public, max-age=31536000, immutable`                         |
| Conditional asset requests work       | Pass    | ETag + `If-None-Match` returns 304                                           |
| Byte ranges work                      | Pass    | `Range: bytes=0-6` returns 206 with `Content-Range`                          |
| Invalid ranges are controlled         | Pass    | Parser returns 416 with `Content-Range: bytes */len`                         |
| Dynamic HTML is not cached by default | Pass    | Gateway adds `no-store` when Worker omits cache headers                      |
| Old asset versions survive draining   | Pass    | Retired asset-root fallback                                                  |
| Preview bypasses caching              | Partial | Legacy preview works; universal preview token propagation is not implemented |
| Compression (Brotli/gzip)             | Not yet | Rust serves identity bytes                                                   |
| Asset garbage collection              | Not yet | Retired roots accumulate; Phase 10 review noted this                         |
| CDN-specific cache keys               | Not yet | No `Vary`/`Cache-Tag` contract                                               |

## Findings

1. **The ETag is metadata-based, not content-based.** Artifact assets are
   immutable per digest, so size+mtime is sufficient in practice; a content hash
   would be stronger if files are ever rewritten in place.
2. **`no-store` is a conservative default.** Storefront HTML may include cart and
   customer state; a Worker must explicitly opt into shared caching. No
   invalidation protocol exists yet.
3. **Preview is still the legacy surface.** `/preview` renders through the old
   runtime; a universal preview needs a signed context header and a
   `Cache-Control: private, no-store` guarantee.
4. **No compression or early hints.** The immutable assets are large JS bundles;
   Brotli/gzip precompression or a CDN is the next step.
5. **Range parsing handles one range only.** Multipart ranges are rejected with
   416 rather than implemented; acceptable for static JS/CSS/images.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 12.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.

## Gate decision

The phased implementation is complete for the trusted single-host runtime.
Remaining work is hardening/productization: universal preview, compression/CDN,
per-deployment credentials, metrics, and asset retention.
