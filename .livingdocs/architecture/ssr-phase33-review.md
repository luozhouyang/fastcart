---
type: Architecture Review
title: Universal SSR Runtime Phase 33 Review
description: Cross-origin admin preview cookie review.
kind: review
timestamp: 2026-11-10T10:00:00Z
---

# Universal SSR Runtime Phase 33 Review

## Scope

Phase 33 makes the admin's cross-origin preview iframe work with preview assets:

- The admin page editor already builds the iframe URL on the store's primary
  domain (`previewOriginFor` in `apps/admin/src/pages/page-editor.tsx`).
- The preview response now sets the asset cookie as
  `SameSite=None; Secure; Partitioned` on HTTPS.
- HTTP development keeps `SameSite=Lax` without `Secure`/`Partitioned`.
- The cookie remains `HttpOnly`, path-scoped to
  `/_fastcart/preview-assets`, and carries only the signed preview token.
- `preview_frame_ancestors` continues to control which admin origins may frame
  the preview.

## Acceptance checklist

| Criterion                                    | Status    | Evidence                                                  |
| -------------------------------------------- | --------- | --------------------------------------------------------- |
| HTTPS cookie allows cross-site iframe assets | Pass      | API test asserts `SameSite=None`, `Secure`, `Partitioned` |
| HTTP cookie remains usable locally           | Pass      | API test asserts `SameSite=Lax`, no `Secure`              |
| Cookie stays HttpOnly and path-scoped        | Pass      | Existing assertions                                       |
| Admin iframe targets store origin            | Pass      | Existing `previewOriginFor` uses `store.primary_domain`   |
| Frame ancestry remains configurable          | Pass      | Existing CSP `preview_frame_ancestors`                    |
| Third-party cookie blocking                  | Partial   | CHIPS helps Chrome; Safari/Firefox may block              |
| Top-level preview fallback                   | Available | `window.open`/new tab uses a first-party context          |

## Findings

1. **Cross-origin preview requires the store origin.** If the iframe points at the
   admin origin, relative preview assets resolve there; the admin already builds
   the iframe from the store domain.
2. **CHIPS is not universal.** `Partitioned` and third-party cookies are blocked
   by some browser/privacy configurations; a top-level `/preview` open is the
   reliable fallback.
3. **`SameSite=None` broadens cookie attachment.** It is limited to
   `/_fastcart/preview-assets`, is HttpOnly, and only authorizes read-only
   preview assets; the signed token and store binding remain enforced.
4. **HTTPS is required for cross-site cookies.** Local HTTP keeps `SameSite=Lax`;
   browsers may reject a `Secure` cookie on HTTP.
5. **Frame ancestors are separate.** `preview_frame_ancestors` must include the
   admin origin; the cookie change does not affect CSP.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 33.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the cross-origin cookie and browser caveat.

## Gate decision

Cross-origin preview assets work in CHIPS-capable browsers and retain a
first-party fallback. Third-party-cookie-blocked browsers remain a documented
limitation.
