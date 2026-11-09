# @fastcart/storefront-build

Packager for the FastCart SSR Artifact contract. The manifest schema lives in
[`schemas/storefront-artifact/manifest.schema.json`](../../schemas/storefront-artifact/manifest.schema.json).

## Usage

```ts
import { createManifest, packageArtifact } from "@fastcart/storefront-build";

const manifest = createManifest({
  artifactId: "my-storefront",
  framework: { name: "tanstack-start", version: "1.169.5" },
  entry: { type: "worker", path: "worker/index.js", export: "default" },
  assets: { directory: "client", base_path: "/_fastcart/assets/" },
});

const packaged = await packageArtifact({
  sourceDir: "./dist",
  outputDir: "./artifacts",
  manifest,
});
```

`packageArtifact` validates the manifest and source tree, rejects symlinks,
traversal, missing entries/assets and configured size limits, writes
`fastcart-ssr.json`, computes the SHA-256 tree digest and stores the artifact at
`<outputDir>/<digest>`. Importing identical content twice is idempotent.

The TypeScript API and the Rust parser share the same field names and capability
list; keep both in sync when the schema version changes.
