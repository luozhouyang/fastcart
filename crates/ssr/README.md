# fastcart-ssr

Deployment lifecycle and request serving for the universal storefront SSR
runtime.

This is the product layer. It answers _what runs for a store, and when_:

- the deployment registry keyed by store, with activation, rollback history and
  atomic persistence under `<runtime_dir>/deployments.json`;
- preview deployments with their own artifact, credentials and asset prefix;
- runtime groups, drain-on-replace and artifact reference counting;
- the storefront response cache and its invalidation rules;
- static asset serving (range requests, content types, preview authorization);
- [`SsrService`](src/service.rs), the single entry point, plus the
  `UniversalStorefront` contract the HTTP layer depends on.

`SsrService` is the service, not an execution runtime: it never spawns or
supervises a process itself. `RuntimeManager` in `fastcart-ssr-workerd` does
that, one workerd process per runtime group.

It knows nothing about how `workerd` is configured or supervised. That is
[`fastcart-ssr-workerd`](../ssr-workerd), which owns the process supervisor, the
loopback gateway, `config.capnp` generation and the `fastcart-ssr.json` artifact
contract.

## Boundaries

| Concern                                                              | Crate                              |
| -------------------------------------------------------------------- | ---------------------------------- |
| Deploy, activate, roll back, preview, cache, serve assets            | `fastcart-ssr`                     |
| Generate workerd config, spawn/health-check/kill workerd, proxy HTTP | `fastcart-ssr-workerd`             |
| Store-scoped internal credentials                                    | `fastcart-ssr-credentials`         |
| Store resolution, trusted context injection, public HTTP surface     | `fastcart-api` / `fastcart-server` |

The dependency direction is one-way: `fastcart-ssr -> fastcart-ssr-workerd`.
`fastcart-ssr-workerd` never depends on stores, deployments or this crate.

## Testing

```bash
cargo test -p fastcart-ssr
cargo clippy -p fastcart-ssr --all-targets -- -D warnings
```

The integration tests render through a real workerd process. They look for
`experiments/workerd/bin/workerd` or `FASTCART_WORKERD_BIN` and skip when no
binary is available.
