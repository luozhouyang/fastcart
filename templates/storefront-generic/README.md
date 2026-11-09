# FastCart generic Fetch Handler storefront

Minimal framework-free Worker proving the universal runtime contract. It has no
framework adapter and no Node-only runtime dependency.

```bash
pnpm install
pnpm build:artifact
pnpm test:artifact
```

The artifact contains `worker/index.js` and `client/app.js`, with
`fastcart-ssr.json` written by `@fastcart/storefront-build`.
