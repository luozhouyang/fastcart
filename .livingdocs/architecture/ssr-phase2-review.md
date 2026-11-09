---
type: Architecture Review
title: Universal SSR Runtime Phase 2 Review
description: Artifact contract, packager and content-addressed storage acceptance review.
kind: review
timestamp: 2026-11-09T04:30:00Z
---

# Universal SSR Runtime Phase 2 Review

## Scope

Phase 2 defines the boundary between a framework build and the runtime:

- `schemas/storefront-artifact/manifest.schema.json` — versioned manifest.
- `packages/storefront-build` — TypeScript manifest builder, validator, digest
  calculator and packager.
- `crates/fastcart-ssr/src/artifact` — Rust parser, validator, digest and
  content-addressed store.

## Acceptance checklist

| Criterion                                              | Status  | Evidence                                                                                                |
| ------------------------------------------------------ | ------- | ------------------------------------------------------------------------------------------------------- |
| Manifest can be generated                              | Pass    | `createManifest` in the packager tests                                                                  |
| Manifest can be parsed by Rust                         | Pass    | `the_checked_in_example_matches_the_rust_manifest`                                                      |
| Invalid manifests are rejected                         | Pass    | Unsupported engine/capability, duplicate flags, bad timestamps/paths                                    |
| Missing entrypoints are rejected                       | Pass    | Rust `MissingEntry` and packager test                                                                   |
| Path traversal is rejected                             | Pass    | Rust `UnsafePath`, schema negative lookahead and packager test                                          |
| Artifact digest is verified                            | Pass    | `ArtifactStore::load` recomputes and compares the digest                                                |
| Artifact is stored immutably by digest                 | Partial | Content is stored at `<digest>` and never overwritten; filesystem read-only permissions are not set yet |
| Rust and TypeScript agree on digest                    | Pass    | Both tests assert `2f7c7d75...e10b` for the same fixture                                                |
| Symlinks are rejected                                  | Pass    | Rust and packager walkers reject symlinks                                                               |
| Size and file limits are enforced                      | Pass    | `ArtifactLimits` in Rust; `DEFAULT_LIMITS` in TypeScript                                                |
| Example validates against the schema's required fields | Pass    | `the_json_schema_and_rust_required_fields_agree`                                                        |

## Findings

1. **Digest ordering was fixed during review.** The first TypeScript
   implementation sorted directory entries during the walk but did not sort the
   final relative-path list; Rust sorts globally. A cross-language fixture now
   pins the algorithm so this cannot regress.
2. **Immutability is logical, not OS-enforced.** Artifacts are addressed by
   digest and imports never overwrite an existing digest, but files are not yet
   chmod-read-only. Adding read-only permissions changes cleanup and rollback
   behavior, so it belongs with deployment activation in Phase 9.
3. **The schema is advisory for Rust.** Rust performs the authoritative
   validation; the JSON Schema mirrors the rules for external producers. The
   checked-in example is parsed by Rust so the two cannot drift silently on
   required fields.
4. **No archive layer.** Phase 2 stores an extracted directory. A compressed
   archive/upload format can be added without changing the manifest or digest
   contract.
5. **Framework adapters are not wired.** The packager is generic; Phase 5 and
   Phase 11 produce the TanStack Start and Astro source layouts that feed it.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-ssr
CARGO_HOME="$PWD/.cargo-home" cargo clippy -p fastcart-ssr --all-targets -- -D warnings
CI=true pnpm --filter @fastcart/storefront-build typecheck
CI=true pnpm --filter @fastcart/storefront-build test
CI=true pnpm lint
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 2
  as implemented and the overall implementation as partial.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- `crates/fastcart-ssr/README.md` and `packages/storefront-build/README.md`
  describe the artifact boundary.

## Gate decision

Phase 3 (runtime manager and configuration generator) may start. The manifest,
digest and storage contract is stable enough for deployment records and
supervision to reference it.
