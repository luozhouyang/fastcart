---
type: Architecture Review
title: Universal SSR Runtime Phase 8 Review
description: Multi-store deployment registry review.
kind: review
timestamp: 2026-11-09T09:00:00Z
---

# Universal SSR Runtime Phase 8 Review

## Scope

Phase 8 makes the universal runtime multi-store aware:

- `SsrConfig.deployments` is a persisted, config-backed registry with one
  `store_id`, `deployment_id`, artifact directory and loopback listener per
  entry.
- `UniversalSsr` holds one isolated workerd group and gateway per store.
- Storefront requests select the deployment by the resolved `store_id`; there is
  no process-global active deployment.
- `/_fastcart/assets/*` resolves the store from `Host` and serves that store's
  artifact client directory.
- An empty `deployments` list preserves the single default-artifact mode.

## Acceptance checklist

| Criterion                              | Status  | Evidence                                                                      |
| -------------------------------------- | ------- | ----------------------------------------------------------------------------- |
| Server can host multiple stores        | Pass    | Two-store registry starts two workerd groups                                  |
| Store resolves to the correct artifact | Pass    | Each group's worker echoes its trusted store and deployment id                |
| Concurrent stores are isolated         | Partial | Separate groups/assets proven sequentially; no load test yet                  |
| No mutable global active deployment    | Pass    | Selection is a registry lookup by resolved store id                           |
| Missing store is controlled            | Pass    | Unknown store returns 404 instead of another artifact                         |
| Assets follow the store                | Pass    | Per-store `app.js` contents differ and are served from the matching artifact  |
| Config is validated                    | Pass    | Duplicate store ids/ports, non-loopback listeners and unsafe ids are rejected |
| Deployment registry is persisted       | Partial | Config file only; database-backed activation is Phase 9                       |
| Hot replacement/rollback               | Not yet | Phase 9                                                                       |
| Per-store internal tokens              | Not yet | One configured token still authenticates every store; Phase 9                 |

## Findings

1. **Ports are explicit or offset from `ssr.listen`.** When a deployment omits
   `listen`, the server uses `ssr.listen.port() + index`. This is deterministic
   but can collide with other services; production configs should set every
   listener explicitly.
2. **All groups share one `RuntimeManager`.** This keeps startup/shutdown
   coordinated but means a manager-level lock serializes management operations.
   Requests use independent gateways and are not serialized by the lock.
3. **The registry is boot-time state.** There is no activation API, health-based
   draining or rollback yet; those are Phase 9.
4. **Internal commerce authentication is still process-wide.** The gateway
   injects the same token for every store. Phase 9 must scope credentials to a
   deployment/store and rotate them.
5. **Asset resolution adds a platform store lookup per request.** The resolver
   cache bounds the cost; Phase 12 can add an asset-specific cache key.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib config
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 8.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and `config.example.toml` document `[[ssr.deployments]]`.

## Gate decision

Phase 9 (activation, replacement, rollback and draining) may start. The registry
provides the stable per-store boundary it will mutate.
