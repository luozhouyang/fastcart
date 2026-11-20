# Plugin System P1 Report: Manifest, Artifacts, Registry and Admin API

**Date:** 2026-11-10
**Status:** Complete. All P1 exit criteria hold; three deviations are recorded below.
**Companion documents:** [Plugin System design](workerd-plugin-system.md), [P1 plan](plugin-p1-plan.md), [P0 report](plugin-p0-report.md).

---

## 1. Delivered

### Crate `crates/plugin`

| Module                | Contents                                                                                                                               |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `manifest.rs`         | `PluginManifest` v2, structural validation, archive reference validation, settings validation, path/hostname/cron rules                |
| `capability.rs`       | `Scope` enum (15 read/UI scopes), reserved write scopes rejected                                                                       |
| `targets.rs`          | Six enabled admin targets, twelve reserved targets, per-target permission                                                              |
| `artifact/archive.rs` | Safe gzip+tar reader with entry, file, compressed and uncompressed limits; rejects traversal, symlinks, duplicates and special entries |
| `artifact/digest.rs`  | Canonical SHA-256 over sorted path/content pairs                                                                                       |
| `artifact/store.rs`   | Content-addressed install, digest verification on read, bundle construction, artifact listing; files written read-only                 |
| `bundle.rs`           | `RuntimeBundle` JSON shape consumed by the P2 loader                                                                                   |
| `service.rs`          | Install, upgrade, enable, disable, settings, rollback, remove, bundle; one transaction per mutation with an audit outbox entry         |
| `model.rs`            | `InstalledPlugin`, `PluginDetail`                                                                                                      |

The old in-memory `PluginRegistry` and its coarse `Capability` enum were removed; see deviation D2.

### Migrations

| File                                              | Contents                                                                                                            |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `migrations/platform/0002_plugin_permissions.sql` | `plugins.read`, `plugins.write`, `plugins.install`; granted to owner and admin                                      |
| `migrations/store/0048_plugins.sql`               | Installations, secrets, KV, event subscriptions, deliveries, jobs and logs (only installations are exercised in P1) |

### HTTP surface

| Method | Path                                                                | Permission                    |
| ------ | ------------------------------------------------------------------- | ----------------------------- |
| GET    | `/api/v1/admin/plugins`                                             | `plugins.read`                |
| GET    | `/api/v1/admin/plugins/{id}`                                        | `plugins.read`                |
| POST   | `/api/v1/admin/plugins/install` (multipart `archive`)               | `plugins.install`             |
| POST   | `/api/v1/admin/plugins/{id}/upgrade?accept_new_scopes=` (multipart) | `plugins.install`             |
| POST   | `/api/v1/admin/plugins/{id}/enable`                                 | `plugins.write`               |
| POST   | `/api/v1/admin/plugins/{id}/disable`                                | `plugins.write`               |
| PATCH  | `/api/v1/admin/plugins/{id}/settings`                               | `plugins.write`               |
| POST   | `/api/v1/admin/plugins/{id}/rollback`                               | `plugins.write`               |
| DELETE | `/api/v1/admin/plugins/{id}`                                        | `plugins.install`             |
| GET    | `/internal/plugins/v1/stores/{store_id}/bundles/{id}?digest=`       | bearer `plugin_runtime_token` |

Multipart routes are mounted outside the JSON body limit with a dedicated 20 MiB cap, following the existing asset upload pattern.

### Configuration

`[plugins]` in `config/config.toml` and the matching environment variables:

| Key                 | Env                             | Default                      |
| ------------------- | ------------------------------- | ---------------------------- |
| `enabled`           | `FASTCART_PLUGINS_ENABLED`      | `false`                      |
| `root`              | `FASTCART_PLUGINS_ROOT`         | `./data/plugins`             |
| `max_archive_bytes` | —                               | 20 MiB                       |
| `max_module_bytes`  | —                               | 4 MiB                        |
| `runtime_token`     | `FASTCART_PLUGIN_RUNTIME_TOKEN` | unset (endpoint not mounted) |

`config/config.example.toml` documents the section.

### Contract artifacts

- `schemas/plugin-manifest/manifest.schema.json` (JSON Schema 2020-12).
- `schemas/plugin-manifest/examples/chat-widget.json`, validated by a unit test against both the Rust validator and the JSON Schema.

---

## 2. Verification

| Suite                                                                              | Result                                                                             |
| ---------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| `cargo test -p fastcart-plugin`                                                    | 19 passed (9 manifest, 10 archive)                                                 |
| `cargo test -p fastcart-api --test admin_plugins`                                  | 10 passed                                                                          |
| `cargo test -p fastcart-api` (all suites)                                          | passed; OpenAPI counts updated to 193 admin paths, 264 total paths, 373 operations |
| `cargo test -p fastcart-infrastructure`                                            | 34 passed                                                                          |
| `cargo clippy -p fastcart-plugin -p fastcart-api -p fastcart-server --all-targets` | clean                                                                              |
| `cargo fmt --all -- --check`                                                       | clean                                                                              |

Covered behaviors:

- install → list → enable → bundle → disable → delete, including 201/204/409 transitions;
- duplicate install returns 409 and does not create a second artifact;
- traversal, symlink, duplicate entry, missing entry module, oversized module and write scope are rejected before anything is written;
- digest is stable across archive order and detects post-install tampering;
- upgrade with a new scope returns 409 until `accept_new_scopes=true`, then sets `previous_digest`;
- rollback restores the previous digest and version;
- settings are schema-validated and revisioned;
- bundle requires the runtime token and refuses disabled plugins.

---

## 3. Deviations from the Plan

### D1. Store migration is `0048`, not `0037`

`0037_preview_token_revocations.sql` already exists. The plugin migration follows the existing sequence as `0048_plugins.sql`. Cargo embeds migrations at compile time, so adding the file requires rebuilding `fastcart-infrastructure` in development.

### D2. The in-memory `PluginRegistry` was removed

The v2 manifest has no dependency declarations, and every installation is per store. Keeping an in-memory registry alongside the database would create two sources of truth. The database is now the registry. If plugin dependencies are added later, they need a migration plus service logic.

### D3. Bundle endpoint is store-scoped

The P1 plan mentioned a store-less bundle endpoint. Installations are per store, so the endpoint is `/internal/plugins/v1/stores/{store_id}/bundles/{id}`, mirroring the SSR internal namespace. The P2 loader reads `store_id` from the request context, never from a plugin.

---

## 4. Post-Review Fixes (2026-11-10)

The gaps recorded after the first P1 pass were closed in the same phase:

| Gap                                      | Resolution                                                                                                                                                                                                                                   |
| ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No permissions-denied test               | Added `users_without_plugin_permissions_are_forbidden`: a `support-agent` member gets 403 on list and install                                                                                                                                |
| `plugins.enabled=false` was not enforced | Install, upgrade, and enable now return 409 when disabled; list, get, settings, disable, rollback, and delete stay available so operators can clean up; the internal runtime endpoint mounts only when enabled **and** a token is configured |
| Artifact GC missing                      | `ArtifactStore::prune(referenced, min_age)` deletes unreferenced digests; the maintenance loop runs a full sweep every 360 ticks (about one hour) with a one-hour grace period to avoid racing in-flight installs                            |
| Failed installs not audited              | `PluginService::record_failure` appends `plugin.install_failed` / `plugin.upgrade_failed` to `audit_outbox` on a best-effort basis                                                                                                           |
| Rejected upgrade left an orphan artifact | `ArtifactStore::inspect` validates the archive and computes the digest **before** anything is written; install and upgrade inspect first, so duplicate installs and scope rejections no longer store an artifact                             |
| Orphan artifacts from a failed DB write  | Still possible in principle; the hourly GC with the grace period removes them                                                                                                                                                                |

Remaining, intentionally deferred:

| Gap                                                           | Impact                                                  | Owner                           |
| ------------------------------------------------------------- | ------------------------------------------------------- | ------------------------------- |
| No pagination on the plugin list                              | Acceptable for small installations                      | P3 if needed                    |
| `worker_threads` behavior under `nodejs_compat` is unverified | Documented in the P0 report                             | P2                              |
| Manifest schema and Rust validator are maintained separately  | The published example test mitigates drift              | P7 add a generated-schema check |
| Upgrade inspection parses the archive twice                   | Small CPU cost for safety; revisit only if uploads grow | P7                              |

---

## 5. Follow-ups for P2

1. Start the loader gateway and serve `RuntimeBundle` through it.
2. Mint the per-request context token and wire the shared bridge.
3. Enforce the watchdog: deadline, 504, restart with metric and audit.
4. Add per-plugin concurrency semaphore and subrequest counter.
5. Exercise the artifact sweep and failure auditing in P2 integration tests (they are unit-tested at the store/service boundary).
6. Keep the artifact directory creation on first install; decide whether startup should fail when `enabled=true` and the root is unwritable.
