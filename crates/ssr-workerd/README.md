# fastcart-ssr-workerd

workerd execution layer for the universal storefront SSR runtime.

This crate answers one question: _how do we run one FastCart SSR artifact in a
supervised `workerd` process?_ It knows nothing about stores, deployments,
activation or previews. Those belong to [`fastcart-ssr`](../ssr), which builds on
this crate.

## Process control

- `WorkerdProcess`: spawn `workerd serve`, capture stdout/stderr, wait for a
  configurable readiness endpoint, report health, send SIGTERM and force-kill
  after the shutdown deadline.
- `SsrGateway`: forward an `http::Request<Bytes>` to workerd over loopback,
  remove hop-by-hop headers and expose the response body as a stream.
- `WorkerdConfig`: binary/config paths, listen address, health path and
  timeouts.

## Runtime manager

`RuntimeManager` starts one workerd process per runtime group, generates
`groups/<id>/config.capnp` from a stored artifact, probes
`/__fastcart/health`, applies a restart backoff and enforces `max_restarts`.
Group state, restart counters and logs are currently in memory; the persisted
deployment registry lives in `fastcart-ssr`.

## Artifact contract

`fastcart_ssr_workerd::artifact` parses and validates `fastcart-ssr.json`,
computes a deterministic SHA-256 tree digest, rejects
traversal/symlinks/missing entry or assets, and stores artifacts
content-addressed under `<store>/<digest>`. The matching TypeScript packager
lives in [`packages/storefront-build`](../../packages/storefront-build) and the
JSON Schema is in [`schemas/storefront-artifact`](../../schemas/storefront-artifact).

## Testing

```bash
bash scripts/install-workerd.sh
cargo test -p fastcart-ssr-workerd
cargo clippy -p fastcart-ssr-workerd --all-targets -- -D warnings
```

The integration test looks for `experiments/workerd/bin/workerd` or
`FASTCART_WORKERD_BIN`. It skips when no binary is available, so
`cargo test --workspace` remains runnable in CI without the 152 MB runtime.
