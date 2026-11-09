import { cp, mkdir, rm } from "node:fs/promises";
import { join } from "node:path";

import { createManifest, packageArtifact } from "../../../packages/storefront-build/src/index.ts";

const root = new URL("..", import.meta.url).pathname;
const dist = join(root, "dist");
const source = join(dist, "artifact");
const output = join(dist, "artifacts");

await rm(source, { recursive: true, force: true });
await rm(output, { recursive: true, force: true });
await mkdir(source, { recursive: true });
await cp(join(dist, "server"), join(source, "worker"), { recursive: true });
await cp(join(dist, "client"), join(source, "client"), { recursive: true });

const manifest = createManifest({
  artifactId: "tanstack-fixture",
  framework: { name: "tanstack-start", version: "1.169.5" },
  entry: { type: "worker", path: "worker/index.js", export: "default" },
  assets: { directory: "client", base_path: "/_fastcart/assets/" },
  runtime: {
    required_capabilities: ["fetch", "streams", "web-crypto"],
  },
  rendering: { mode: "hybrid", streaming: true },
  build: { build_id: "tanstack-fixture" },
});

const packaged = await packageArtifact({
  sourceDir: source,
  outputDir: output,
  manifest,
});

console.log(`artifact=${packaged.directory}`);
console.log(`digest=${packaged.digest}`);
