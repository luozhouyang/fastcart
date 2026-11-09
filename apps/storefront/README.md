# Fastcart Storefront (development harness)

This app is a development harness and reference for the first framework target
of the universal SSR runtime. It is not the production storefront yet.

The target architecture runs framework-built Fetch-handler artifacts in a
process-isolated `workerd` runtime managed by `fastcart-serve`. TanStack Start is
the first supported framework; the legacy theme bundles and Node sidecar are
transitional and will be removed after the migration. See
[ADR 0007](../../.livingdocs/adr/0007-universal-ssr-runtime-with-workerd.md) and
the [universal SSR runtime plan](../../.livingdocs/architecture/universal-ssr-runtime.md).

## Current scope

The harness has a small home page and a placeholder product route that does not
load commerce data. Run it with:

```bash
pnpm --filter @fastcart/storefront dev
```

Vite serves it on port 3000.

## Related

- [Universal SSR runtime plan](../../.livingdocs/architecture/universal-ssr-runtime.md).
- [Repository README](../../README.md).
