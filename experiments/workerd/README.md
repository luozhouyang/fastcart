# Phase 0 workerd experiments

These fixtures validate the runtime assumptions behind [ADR 0007](../../.livingdocs/adr/0007-universal-ssr-runtime-with-workerd.md)
before the production `fastcart-ssr` crate is written.

## Pinned runtime

- Version: `v1.20261109.1`
- Asset: `workerd-linux-64.gz` for Linux x86_64
- Binary location: `experiments/workerd/bin/workerd` (ignored by Git)
- Install: `bash scripts/install-workerd.sh`

The binary is not committed. The install script verifies the recorded SHA-256
before making it executable.

## Checks

| Fixture                                      | What it proves                                                                                           |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `configs/basic.capnp`                        | A standard ES-module Fetch handler runs and returns a response                                           |
| `configs/web-api.capnp`                      | Request, Response, Headers, URL, streams, encoding, Web Crypto, AbortController and timers are available |
| `configs/streaming.capnp`                    | `ReadableStream` responses reach the client incrementally                                                |
| `configs/service-bindings.capnp`             | A Worker can call another workerd service without a public socket                                        |
| [frameworks/README.md](frameworks/README.md) | TanStack Start and Astro output both run as Fetch handlers under workerd                                 |

Run:

```bash
bash scripts/install-workerd.sh
bash experiments/workerd/test/phase0.sh
```

The test starts each fixture on a loopback port (`18787`–`18790`), asserts the
response, and stops the process. Logs go to `${TMPDIR:-/tmp}/fastcart-workerd-phase0`.

## Phase 0 result

Validated on 2026-11-09 with workerd `v1.20261109.1`:

- Basic Fetch handler, Request/Response/Headers/URL, streams, encoding, Web
  Crypto, AbortController, timers and service bindings pass.
- Streaming is incremental: three chunks arrived about 300 ms apart.
- TanStack Start (`1.169.5`) and Astro (`7.4.7`) SSR output both served a real
  document through workerd. See [frameworks/README.md](frameworks/README.md)
  for versions, build commands and caveats.

Phase 0 does not implement the artifact packager, runtime supervisor, gateway,
static asset serving or deployment registry. Those start in Phase 1 with
`crates/fastcart-ssr`.
