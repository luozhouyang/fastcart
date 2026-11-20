# Admin panel plugin

`fastcart-plugin pack .` bundles `worker/index.ts` with esbuild and copies
`ui/` into the artifact. The host serves the HTML in a sandboxed iframe and
proxies `panel.info` through the admin RPC endpoint.
