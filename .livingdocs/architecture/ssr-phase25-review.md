---
type: Architecture Review
title: Universal SSR Runtime Phase 25 Review
description: Credential backend metrics and encryption-at-rest review.
kind: review
timestamp: 2026-11-10T02:00:00Z
---

# Universal SSR Runtime Phase 25 Review

## Scope

Phase 25 adds two hardening layers to the Phase 24 credential backend:

- Low-cardinality Prometheus metrics for credential load, save, refresh and live
  token count, plus the last successful backend operation.
- Optional XChaCha20-Poly1305 encryption of the file backend, with the key read
  from an environment variable and never logged.
- Legacy plaintext files remain readable; the next write uses the encrypted
  envelope.
- The existing `0600` mode and atomic/locked write path remain in force.

## Acceptance checklist

| Criterion                                  | Status          | Evidence                                     |
| ------------------------------------------ | --------------- | -------------------------------------------- |
| Load/save/refresh metrics render           | Pass            | Manager metrics test                         |
| Token count and last-success gauges        | Pass            | Manager metrics test                         |
| Metrics stay low-cardinality               | Pass            | No store/backend/path labels                 |
| Encrypted roundtrip                        | Pass            | Encrypted backend test                       |
| Plaintext token absent from encrypted file | Pass            | Raw file assertion                           |
| Wrong key is rejected                      | Pass            | Decryption error test                        |
| Legacy plaintext remains readable          | Pass            | Legacy load test                             |
| Key is sourced from env                    | Pass            | `key_env` config, startup read               |
| Key never logged/Debug-redacted            | Pass            | Backend Debug prints only `encrypted`        |
| Key rotation                               | Not yet         | No multi-key/key-id support                  |
| Automatic plaintext migration on startup   | Not yet         | Encrypts on next write                       |
| External KMS/secret manager                | Not implemented | Out of scope                                 |
| Metrics for decryption failures            | Partial         | Reported as load errors, no separate counter |

## Findings

1. **Encryption is opt-in.** Existing deployments continue with plaintext unless
   `encryption = "xchacha20poly1305"` and `key_env` are configured.
2. **The encryption key is not in TOML.** It is read from an environment variable
   (default `FASTCART_SSR_CREDENTIALS_KEY`) as base64; the process environment is
   still sensitive and should be protected by the orchestrator.
3. **Legacy plaintext is accepted.** An encrypted deployment can read an old
   plaintext file, but only rewrites it encrypted after rotation/revocation or
   seeding. Operators should trigger a write after enabling encryption.
4. **No key rotation.** A single key is used; changing it makes the existing file
   unreadable. A key-id envelope is future work.
5. **Metrics have no backend label.** Only the file backend exists, so this is
   deliberate; adding a backend later should use a bounded label set.
6. **Refresh metrics count load attempts too.** Refresh failures are recorded in
   both load and refresh counters; dashboards should use the refresh counters
   for refresh health.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-ssr-credentials
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-http
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib config::tests::example_config_parses
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 25.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and `config.example.toml` document encryption and key handling.

## Gate decision

Credential metrics and opt-in encryption are implemented. Key rotation, KMS
integration and automatic migration remain open.
