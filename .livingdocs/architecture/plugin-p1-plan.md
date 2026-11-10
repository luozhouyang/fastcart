# Plugin System P1: Manifest, Artifacts, Registry and Admin API

**Status:** Implemented 2026-11-10. Results and deviations are in [plugin-p1-report.md](plugin-p1-report.md).
**Depends on:** [Plugin System design](workerd-plugin-system.md), [P0 report](plugin-p0-report.md).
**Duration:** ~7 person-days.
**Goal:** a merchant can install, inspect, enable, disable, configure, upgrade, roll back, and remove a plugin; the host validates every artifact and can serve the plugin's module graph to the runtime in P2. No workerd process is started in P1.

---

## 1. Scope

### In scope

- `plugin.json` v2 schema and a Rust validator with target and scope gates.
- Content-addressed artifact storage (`data/plugins/artifacts/<digest>/`) and safe archive extraction.
- Store-schema migration for installations, settings, secrets, KV, event subscriptions, deliveries, jobs and logs (only installations/settings/secrets are exercised in P1).
- `fastcart-plugin` service layer: install, upgrade, enable, disable, settings, rollback, delete, bundle read.
- Admin HTTP API under `/api/v1/admin/plugins`.
- Internal bundle delivery endpoint under `/internal/plugins/v1/runtime/bundles/{id}`.
- RBAC permissions `plugins.read`, `plugins.write`, `plugins.install` and audit records.
- Unit and integration tests, including a tar.gz fixture builder.

### Out of scope (later phases)

- Starting or supervising workerd (P2).
- KV API, egress proxy, token minting, events and jobs (P2/P5).
- Admin UI target rendering and the plugin management page (P3).
- Storefront injection and assets (P4).
- Artifact garbage collection, signing, revocation (P6/P8).
- The `@fastcart/plugin-cli` packer (P7); P1 tests build archives in Rust.

### Exit criteria

1. `cargo test -p fastcart-plugin` and `cargo test -p fastcart-api --test admin_plugins` pass.
2. A malformed archive (traversal, symlink, oversized, missing entry, bad manifest, reserved target, write scope) is rejected with a stable error code and never written under the artifact root.
3. Install → enable → upgrade → rollback → disable → delete works end to end through HTTP.
4. Every mutating endpoint writes an audit record with the acting user.
5. The bundle endpoint returns exactly the modules stored for the requested digest and rejects an unknown digest or a wrong token.
6. `cargo clippy --workspace --all-targets` and the existing test suite stay green.

---

## 2. Dependencies

Add to the workspace and to `crates/plugin`:

| Crate                     | Use                                   | Availability                     |
| ------------------------- | ------------------------------------- | -------------------------------- |
| `sha2`                    | digest                                | already a workspace dependency   |
| `tar` 0.4                 | archive reading                       | present in the local cargo cache |
| `flate2` 1.1              | gzip decoding                         | present in the local cargo cache |
| `jsonschema` 0.33         | settings schema validation            | already in `Cargo.lock`          |
| `semver` 1.0              | manifest version parsing              | already in `Cargo.lock`          |
| `fastcart-infrastructure` | `StoreDatabase`, `AuditActor`, errors | workspace                        |
| `fastcart-kernel`         | ids, `AppError`                       | workspace                        |
| `sqlx`                    | store queries                         | workspace                        |
| `utoipa` / `utoipa-axum`  | OpenAPI                               | workspace                        |

If `tar` or `flate2` cannot be locked offline, fall back to a JSON bundle format (`plugin.json` plus base64 modules) and keep the same validation API; record the change in the design doc.

---

## 3. File-Level Plan

### 3.1 `crates/plugin`

```text
crates/plugin/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── error.rs
│   ├── manifest.rs
│   ├── capability.rs
│   ├── targets.rs
│   ├── registry.rs
│   ├── artifact/
│   │   ├── mod.rs
│   │   ├── archive.rs
│   │   ├── digest.rs
│   │   ├── store.rs
│   │   └── validate.rs
│   ├── service.rs
│   ├── bundle.rs
│   └── model.rs
└── tests/
    ├── manifest_validation.rs
    ├── artifact_store.rs
    └── archive_safety.rs
```

| Path                           | Change | Content                                                                                                                                                                                                                                                                                                                             | Acceptance                                                    |
| ------------------------------ | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `Cargo.toml`                   | edit   | Add `sha2`, `tar`, `flate2`, `jsonschema`, `semver`, `sqlx`, `fastcart-infrastructure`, `fastcart-kernel`, `utoipa`, `tracing`, `chrono`. Keep the crate free of `axum`.                                                                                                                                                            | `cargo check -p fastcart-plugin`                              |
| `src/lib.rs`                   | edit   | Re-export `PluginManifest`, `PluginService`, `ArtifactStore`, `PluginError`, `RuntimeBundle`; keep the existing registry API compiling.                                                                                                                                                                                             | existing registry tests pass                                  |
| `src/error.rs`                 | new    | `PluginError` with stable codes: `invalid_manifest`, `unsupported_api`, `unknown_target`, `reserved_target`, `unsupported_scope`, `invalid_archive`, `archive_too_large`, `path_traversal`, `duplicate_plugin`, `not_found`, `disabled`, `settings_invalid`, `digest_mismatch`, `storage`. Map to `AppError` once in the API layer. | unit tests per variant                                        |
| `src/manifest.rs`              | new    | `PluginManifest` v2 (all fields in 5.2), `Entry`, `RuntimeSpec`, `Capabilities`, `AdminExtension`, `StorefrontExtension`, `JobSpec`, `SettingsSpec`, `BuildInfo`. `validate(&self, host: &HostCapabilities)`. `api_version` must be 2; `schema_version` must be `"1"`.                                                              | table-driven tests                                            |
| `src/capability.rs`            | new    | `Scope` enum: `core.context`, `catalog.read`, `orders.read`, `customers.read`, `content.read`, `carts.read`, `kv.read`, `kv.write`, `events.subscribe`, `network.egress`, `jobs.enqueue`, `admin.ui`, `storefront.ui`. Write scopes (`catalog.write`, `orders.write`, …) parse as reserved and fail validation.                     | read scopes pass; write scopes rejected                       |
| `src/targets.rs`               | new    | Enabled target list (six) and reserved list; `Target::parse`, `is_enabled`, `required_permission`.                                                                                                                                                                                                                                  | unknown and reserved targets rejected                         |
| `src/registry.rs`              | move   | Move the current `PluginRegistry` and its tests out of `lib.rs` unchanged.                                                                                                                                                                                                                                                          | existing tests unchanged                                      |
| `src/artifact/archive.rs`      | new    | Streaming tar.gz reader with limits: entries ≤ 2000, single file ≤ 5 MB, total ≤ 20 MB, decompressed ≤ 40 MB. Reject absolute paths, `..`, symlinks, hardlinks, devices, duplicate names, and non-UTF-8 names.                                                                                                                      | corpus: traversal, symlink, bomb, duplicate, oversized        |
| `src/artifact/validate.rs`     | new    | Cross-check the manifest against the archive: entry exists, every `ui/` and `storefront/` reference exists, module count and per-module size (≤ 4 MB), egress host syntax, settings schema is an object.                                                                                                                            | each failure has a stable code                                |
| `src/artifact/digest.rs`       | new    | Canonical digest: SHA-256 over `schema_version`, sorted `(path, sha256, size)` triples, and the manifest bytes. Hex-encoded, 64 chars.                                                                                                                                                                                              | fixed fixture digest                                          |
| `src/artifact/store.rs`        | new    | `ArtifactStore { root, limits }`: `install(bytes) -> StoredArtifact`, `read_bundle(digest) -> RuntimeBundle`, `get(digest)`, `list()`. Extract to `tmp/<uuid>`, validate, `rename` into `artifacts/<digest>`, then `chmod` read-only where supported. Never overwrite an existing digest.                                           | concurrent installs of the same archive produce one directory |
| `src/service.rs`               | new    | `PluginService { db: StoreDatabase, artifacts: ArtifactStore, audit: Option<AuditActor> }` with `new`, `with_audit_actor`, `list`, `get`, `install`, `upgrade`, `enable`, `disable`, `update_settings`, `rollback`, `remove`, `bundle`. Mutations run in one transaction with the audit entry; `bundle` is read-only.               | integration tests                                             |
| `src/bundle.rs`                | new    | `RuntimeBundle { compatibility_date, main_module, modules: BTreeMap<String, ModuleSource>, compatibility_flags, limits: Option<RuntimeLimits>, digest }`, serialized to the JSON shape the P0 gateway consumes.                                                                                                                     | round-trips the P0 fixture shape                              |
| `src/model.rs`                 | new    | Row and DTO types: `InstalledPlugin`, `ArtifactSummary`, `PluginSettings`, `PluginDetail`.                                                                                                                                                                                                                                          | serde round-trip                                              |
| `tests/manifest_validation.rs` | new    | Valid manifest; bad id; bad version; api_version 1 and 3; unknown target; reserved target; write scope; egress host with scheme or IP; settings schema not an object.                                                                                                                                                               | one assertion per rule                                        |
| `tests/artifact_store.rs`      | new    | Install, digest stability, duplicate install, read bundle, unknown digest, rollback digest retained, read-only permissions.                                                                                                                                                                                                         | deterministic                                                 |
| `tests/archive_safety.rs`      | new    | Traversal, absolute path, symlink, hardlink, duplicate entry, total size over limit, entry count over limit, missing entry module.                                                                                                                                                                                                  | each rejected before writing                                  |

### 3.2 Migrations

| Path                                              | Content                                                                                                                                                                                                                                                                                  |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `migrations/platform/0002_plugin_permissions.sql` | Insert `plugins.read`, `plugins.write`, `plugins.install`; grant to the owner and admin roles using the existing role ids.                                                                                                                                                               |
| `migrations/store/0048_plugins.sql`               | All tables from design 10.2: `plugin_installations`, `plugin_secrets`, `plugin_kv`, `plugin_event_subscriptions`, `plugin_deliveries`, `plugin_jobs`, `plugin_logs`. P1 exercises `plugin_installations` and `plugin_secrets`; the rest are created now so later phases do not renumber. |

Notes:

- `plugin_installations` keeps `previous_digest` for rollback and `granted_scopes` for consent drift checks.
- `settings_json` defaults to `{}`; `settings_revision` increments on every accepted update.
- No generic `plugin_records` table (review decision 12).

### 3.3 API

| Path                                               | Change | Content                                                                                                                                             |
| -------------------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/api/src/routes/admin_plugins.rs`           | new    | Admin routes, request/response DTOs, permission checks, multipart install/upgrade.                                                                  |
| `crates/api/src/routes/internal_plugin_runtime.rs` | new    | Bearer-token bundle delivery.                                                                                                                       |
| `crates/api/src/routes/mod.rs`                     | edit   | `pub mod admin_plugins; pub mod internal_plugin_runtime;`                                                                                           |
| `crates/api/src/lib.rs`                            | edit   | Mount `/api/v1/admin/plugins`; add a separate upload router with its own body limit; mount `/internal/plugins/v1/runtime` with a bearer middleware. |
| `crates/api/src/openapi.rs`                        | edit   | Register the admin routes and DTO schemas in the OpenAPI document.                                                                                  |
| `crates/api/src/state.rs`                          | edit   | Add `plugins: Arc<PluginService>` and `plugin_runtime_token: Option<String>` to `AppContext`.                                                       |

Admin endpoints (all under `/api/v1/admin/plugins`, permission in parentheses):

| Method | Path             | Permission        | Behavior                                                            |
| ------ | ---------------- | ----------------- | ------------------------------------------------------------------- |
| GET    | `/`              | `plugins.read`    | Installed plugins plus a summary of artifacts on disk               |
| POST   | `/install`       | `plugins.install` | Multipart `archive`; validate, store, insert disabled               |
| GET    | `/{id}`          | `plugins.read`    | Manifest, settings, digests, timestamps                             |
| POST   | `/{id}/upgrade`  | `plugins.install` | Multipart; same plugin id, new digest, `previous_digest` set        |
| POST   | `/{id}/enable`   | `plugins.write`   | Enable; reject when no artifact or invalid settings                 |
| POST   | `/{id}/disable`  | `plugins.write`   | Disable                                                             |
| PATCH  | `/{id}/settings` | `plugins.write`   | `{ settings }` validated against the manifest schema; bump revision |
| POST   | `/{id}/rollback` | `plugins.write`   | Swap current and previous digest; reject when absent                |
| DELETE | `/{id}`          | `plugins.install` | Only when disabled; delete the row, keep the artifact               |

Internal endpoint:

| Method | Path                                                | Auth                                           | Behavior                                                                                        |
| ------ | --------------------------------------------------- | ---------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| GET    | `/internal/plugins/v1/runtime/bundles/{id}?digest=` | `Authorization: Bearer <plugin_runtime_token>` | Return the bundle for the installed plugin and digest; 404 unknown, 409 disabled, 401 bad token |

Response sketch:

```json
{
  "id": "chat-widget",
  "name": "Live Chat",
  "version": "1.2.0",
  "digest": "ab12…",
  "previous_digest": "9f0c…",
  "enabled": true,
  "settings": { "accent": "#2563eb" },
  "settings_revision": 3,
  "granted_scopes": ["catalog.read", "kv.write"],
  "installed_at": "2026-11-10T10:00:00Z",
  "updated_at": "2026-11-10T10:05:00Z"
}
```

Error mapping:

| Situation                          | Status | Code                                   |
| ---------------------------------- | ------ | -------------------------------------- |
| Invalid manifest or archive        | 400    | `invalid_manifest`, `invalid_archive`  |
| Path traversal, symlink, oversized | 400    | `path_traversal`, `archive_too_large`  |
| Reserved target or write scope     | 400    | `reserved_target`, `unsupported_scope` |
| Duplicate install                  | 409    | `duplicate_plugin`                     |
| Missing plugin or digest           | 404    | `not_found`                            |
| Rollback without a previous digest | 409    | `no_previous_digest`                   |
| Delete while enabled               | 409    | `plugin_enabled`                       |
| Settings schema violation          | 400    | `settings_invalid`                     |
| Missing permission                 | 403    | platform error                         |

### 3.4 Config and server wiring

| Path                                      | Change                                                                                                                                                                                                                                             |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `apps/server/src/config.rs`               | Add `[plugins]`: `enabled` (default false), `root` (default `data/plugins`), `max_archive_bytes` (20 MiB), `max_module_bytes` (4 MiB), `runtime_token`. Env: `FASTCART_PLUGINS_ENABLED`, `FASTCART_PLUGINS_ROOT`, `FASTCART_PLUGIN_RUNTIME_TOKEN`. |
| `apps/server/src/state.rs`                | Build `ArtifactStore` and `PluginService`; keep them in `AppContext`.                                                                                                                                                                              |
| `apps/server/src/bootstrap.rs`            | Create the artifact root with `0700`; fail startup when `enabled` and the root is not writable.                                                                                                                                                    |
| `config/config.example.toml` (if present) | Document the new section.                                                                                                                                                                                                                          |

### 3.5 Schema and fixtures

| Path                                                        | Content                                                                        |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `schemas/plugin-manifest/manifest.schema.json`              | JSON Schema 2020-12 for `plugin.json`; mirrors the Rust model.                 |
| `schemas/plugin-manifest/examples/chat-widget.json`         | Valid example used by tests.                                                   |
| `crates/api/tests/fixtures/plugin_archive.rs` (test helper) | Builds a tar.gz in memory from `(manifest, files)`; used by integration tests. |

---

## 4. Manifest v2 Rules

Validation order matters: reject the cheapest structural errors first, then archive-dependent checks.

1. JSON shape and `additionalProperties: false` (schema).
2. `schema_version == "1"`, `api_version == 2`.
3. `id` pattern and length; `version` semver.
4. `runtime.engine == "workerd"`; `compatibility_date` inside `2025-01-01..=2026-12-31` (host constant); flags allowlist (`nodejs_compat` only in v1).
5. Every admin target is in the enabled set.
6. Every scope is in the v1 read-only set; reserved write scopes produce `unsupported_scope`.
7. Egress hosts: lowercase hostnames, no scheme, no port, no IP literal, at most 10, wildcard only as `*.example.com`.
8. Jobs: unique names, cron subset, at most 10.
9. Archive: entry module exists; UI and storefront entries exist; per-module size ≤ 4 MiB; total ≤ 20 MiB.
10. Settings schema is a JSON object with `type: "object"` when present.

`granted_scopes` is written on install and compared on upgrade; a new scope requires re-enabling through the admin UI (P3). P1 rejects an upgrade that adds scopes unless the request includes `accept_new_scopes=true`.

---

## 5. Test Plan

### Unit (`crates/plugin`)

| Area     | Cases                                                                                                                           |
| -------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Manifest | valid, bad id, bad version, unsupported api, unknown target, reserved target, write scope, bad egress, bad cron, duplicate jobs |
| Archive  | traversal, absolute path, symlink, hardlink, duplicate entry, entry count, file size, total size                                |
| Digest   | stable across file order, changes with content, changes with manifest                                                           |
| Store    | install, duplicate, read bundle, unknown digest, read-only mode                                                                 |
| Service  | enable requires artifact, rollback swaps, delete requires disabled, settings revision increments                                |

### Integration (`crates/api/tests/admin_plugins.rs`)

Follow the existing fixture pattern (store + admin session). Cases:

1. Install a valid archive → 201, row present, artifact directory exists with the expected digest.
2. Install the same archive again → 409.
3. Install an archive with `../` → 400 and no directory under the root.
4. Install a manifest with a write scope → 400.
5. Enable → 200; second enable is idempotent.
6. Upgrade to a new archive → `previous_digest` set; bundle returns the new digest.
7. Rollback → bundle returns the old digest; previous/current swap.
8. Delete while enabled → 409; disable then delete → 204.
9. Settings update with an invalid value → 400; valid → revision increments.
10. User without `plugins.read`/`plugins.write`/`plugins.install` → 403 per endpoint.
11. Bundle endpoint without a token → 401; wrong token → 401; disabled plugin → 409; unknown digest → 404.
12. Audit outbox contains one entry per mutation with the actor id.

---

## 6. Sequence

| Day | Work                                                      | Output                    |
| --- | --------------------------------------------------------- | ------------------------- |
| D1  | Manifest, capability, targets, registry move; schema file | Unit tests for validation |
| D2  | Archive reader, digest, store                             | Archive safety tests      |
| D3  | Migration, `PluginService` CRUD, audit                    | Service tests             |
| D4  | Admin routes, permissions, DTOs, OpenAPI                  | Route tests               |
| D5  | Bundle endpoint, config, server wiring                    | End-to-end bundle read    |
| D6  | Integration tests, archive fixture builder, error mapping | `admin_plugins.rs` green  |
| D7  | Clippy, docs, design-doc updates, review                  | P1 review notes           |

Dependencies: D2 depends on D1 (manifest model); D3 depends on D2; D4/D5 depend on D3; D6 depends on D4/D5.

---

## 7. Risks

| Risk                                                     | Mitigation                                                                                    |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| Tar parser edge cases (bombs, sparse files, PAX headers) | Hard limits before and during extraction; reject unusual entry types; corpus tests            |
| `jsonschema` API drift                                   | Pin 0.33; one wrapper module with a single `validate_settings` function                       |
| Multipart upload buffering                               | Dedicated upload router with a 20 MiB limit, like the asset upload path                       |
| Migration ordering on existing stores                    | Additive migration only; no table rewrites; test on a copy of a migrated store fixture        |
| Permission seeding for existing roles                    | `INSERT OR IGNORE` on permissions, then `INSERT OR IGNORE` on role_permissions by name lookup |
| Artifact disk growth without GC                          | Out of scope for P1; document the manual cleanup path and add GC to P6                        |
| Bundle endpoint token absent                             | Endpoint disabled with 404 when `plugins.enabled` is false or the token is empty              |

---

## 8. Definition of Done

- The six exit criteria in section 1 hold.
- The design doc's phase table marks P1 done and links this plan.
- The API document lists the new endpoints and permissions.
- No workerd process is started, no admin UI is added, and no storefront behavior changes.
- A short P1 report records deviations, test counts, and the first artifact digest fixture.

---

## 9. Follow-ups Created by P1

- P2 consumes `RuntimeBundle` and starts the loader.
- P3 consumes the target list and `granted_scopes` for the consent UI.
- P5 consumes `plugin_event_subscriptions`, `plugin_deliveries`, and `plugin_jobs`.
- P6 adds artifact GC and the `plugins.*` metric families.
- P7 replaces the Rust test archive builder with `@fastcart/plugin-cli pack`.
