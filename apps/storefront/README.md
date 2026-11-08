# Fastcart Storefront (development harness)

This app is a non-production development harness for experimenting with
TanStack Start. It is not the product storefront and is not shipped or required
in production.

Production storefront routes and commerce UI live in theme bundles under
[`themes/`](../../themes/), primarily [`themes/default`](../../themes/default).
Those routes are rendered by the Rust API through the Node sidecar; the embedded
V8 runtime is still a stub. See
[ADR 0005](../../.livingdocs/adr/0005-theme-bundle-as-the-product-storefront.md)
for the decision.

## Current scope

The harness has a small home page and a placeholder product route that does not
load commerce data. Run it with:

```bash
pnpm --filter @fastcart/storefront dev
```

Vite serves it on port 3000.

## Related

- [Theme sidecar](../theme-sidecar/README.md) — the HTTP rendering contract.
- [Repository README](../../README.md).
