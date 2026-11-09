---
type: Architecture Review
title: Universal SSR Runtime Phase 29 Review
description: Preview asset cookie authorization review.
kind: review
timestamp: 2026-11-10T06:00:00Z
---

# Universal SSR Runtime Phase 29 Review

## Scope

Phase 29 restricts `/_fastcart/preview-assets/*`:

- A successful universal preview render sets an HttpOnly `fastcart_preview`
  cookie containing the same signed preview token.
- The cookie is scoped to `Path=/_fastcart/preview-assets`, so it is not sent to
  ordinary storefront or production asset requests.
- The attribute set is `SameSite=Lax`, `Max-Age` until token expiry, and `Secure`
  when the preview request is HTTPS.
- The preview asset route requires:
  1. a configured preview secret,
  2. a valid, unexpired signed token,
  3. a token store matching the Host-resolved store.
- Failures return `403`; a missing active preview deployment still returns the
  normal asset `404`.
- Successful authenticated assets add `Vary: Cookie` to avoid shared-cache reuse.
- The exact `/preview/` route is registered so a trailing slash cannot fall
  through to production rendering.

## Acceptance checklist

| Criterion                           | Status          | Evidence                                                      |
| ----------------------------------- | --------------- | ------------------------------------------------------------- |
| Preview render sets HttpOnly cookie | Pass            | API test checks `set-cookie`, path and `HttpOnly`             |
| Cookie is Secure for HTTPS          | Pass            | Test sends `x-forwarded-proto: https`                         |
| Missing cookie is rejected          | Pass            | Preview asset without cookie returns 403                      |
| Valid cookie serves assets          | Pass            | Preview asset test returns 200                                |
| Cookie adds `Vary: Cookie`          | Pass            | Response header assertion                                     |
| Cross-store token is rejected       | Pass            | `store_other` token against the fixture store returns 403     |
| Expired/invalid token is rejected   | Shared verifier | Reuses `verify_preview_token_unbound` expiry/signature checks |
| `/preview/` does not fall through   | Pass            | Explicit route added; test renders universal preview          |

## Findings

1. **The asset token is the preview token.** No second credential is minted;
   the cookie carries the same signed capability, and the path restriction
   limits its exposure.
2. **Cookie scope is path-based.** Browsers send it only to
   `/_fastcart/preview-assets`; production assets and APIs never receive it.
3. **A missing cookie returns 403.** This is distinguishable from a missing
   preview asset (404), but both reveal little beyond route existence.
4. **Cross-store isolation uses the Host-resolved store.** A cookie minted for
   another store cannot read this store's preview assets.
5. **Shared caches are told about the cookie dependency.** `Vary: Cookie` is
   attached to authenticated asset responses.
6. **Trailing-slash preview URLs were previously falling through.** The
   explicit `/preview/` route closes that gap and is covered by the test.
7. **No cookie revocation beyond token expiry.** Revoking the preview token's
   `jti` still stops new preview pages; an already-issued cookie remains valid
   until token expiry for assets. Phase 30 adds the asset-time revocation lookup.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 29.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents preview asset authorization.

## Gate decision

Preview assets require a store-bound signed cookie. Asset-time JTI revocation
and cross-origin preview hosting remain open.
