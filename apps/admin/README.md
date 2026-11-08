# Fastcart Admin

The web admin for a self-hosted Fastcart store. React 19, TanStack Router, Tailwind CSS 4 and
shadcn/ui primitives on Radix. The visual system is implemented with the UI primitives under
`src/components/ui`; the design notes that informed it are local-only and are not versioned in this
repository.

## Run

```bash
pnpm install
pnpm --filter @fastcart/admin dev
```

The app runs on <http://localhost:5174>. It expects the API on `http://localhost:8080` in
development; set `VITE_FASTCART_API_URL` to point somewhere else.

The API must allow the admin origin with credentials. The example server configuration does this:

```toml
[server]
cookie_secure = false
cors_origins = ["http://localhost:5173", "http://localhost:5174"]
```

Sign in with an account that belongs to a store, or choose **Explore the demo store** on the sign-in
screen. Demo mode keeps the full review dataset in memory and never calls the API.

## Scripts

```bash
pnpm --filter @fastcart/admin dev
pnpm --filter @fastcart/admin build
pnpm --filter @fastcart/admin typecheck
pnpm --filter @fastcart/admin test
```

## Data modes

Both modes feed the same view models in `src/lib/models.ts`.

- **API mode** uses `@fastcart/admin-sdk`, a cookie session, the `x-store-id` header and the CSRF
  token from the `fc_csrf` cookie. Loaders in `src/lib/loaders.ts` normalize minor currency units and
  Unix seconds for every screen.
- **Demo mode** maps `src/data/mock.ts` in `src/lib/demo.ts`. It is the review dataset for Halcyon
  Supply and is explicitly labelled in the shell.

## Notes and limits

- There is no analytics endpoint yet, so the overview aggregates recent orders client-side. Stores
  with more than 500 orders in the window may show a lower revenue total than the true lifetime
  figure.
- Product creation writes a draft product. The admin SDK has no create-price endpoint, so prices are
  managed in price lists outside this screen.
- Orders are created at checkout; the admin can fulfil, refund and inspect them but not create them.
- Customer "new" and "guest" states are derived from join date and account status because the API
  exposes only `active` and `disabled`.
- Domains and store profile edits require `settings.write`; failures surface as toasts.

## Tests

Vitest covers formatting helpers, the demo adapters, API normalization (orders, products, inventory,
customers, settings and mutations), the paged fetch helper and the shared pagination and status
pill components.
