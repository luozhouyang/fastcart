# fastcart-ssr

Process supervisor and HTTP gateway for the universal workerd SSR runtime.

## Phase 1 scope

- `WorkerdProcess`: spawn `workerd serve`, capture stdout/stderr, wait for a
  configurable readiness endpoint, report health, send SIGTERM and force-kill
  after the shutdown deadline.
- `SsrGateway`: forward an `http::Request<Bytes>` to workerd over loopback,
  remove hop-by-hop headers and expose the response body as a stream.
- `WorkerdConfig`: binary/config paths, listen address, health path and
  timeouts.

The crate does not yet know about FastCart artifacts, deployment records,
trusted request context or static assets. Those are later phases of the
[universal SSR runtime](../../.livingdocs/architecture/universal-ssr-runtime.md)
plan.

## Testing

```bash
bash scripts/install-workerd.sh
cargo test -p fastcart-ssr
cargo clippy -p fastcart-ssr --all-targets -- -D warnings
```

The integration test looks for `experiments/workerd/bin/workerd` or
`FASTCART_WORKERD_BIN`. It skips when no binary is available, so
`cargo test --workspace` remains runnable in CI without the 152 MB runtime.
