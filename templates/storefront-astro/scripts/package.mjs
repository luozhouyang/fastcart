import { cp, rename, rm } from "node:fs/promises";
import { join } from "node:path";

import { createManifest, packageArtifact } from "../../../packages/storefront-build/src/index.ts";

const root = new URL("..", import.meta.url).pathname;
const dist = join(root, "dist");
const source = join(dist, "artifact");
const output = join(dist, "artifacts");

await rm(source, { recursive: true, force: true });
await rm(output, { recursive: true, force: true });
await cp(join(dist, "server"), join(source, "worker"), { recursive: true });
await cp(join(dist, "client"), join(source, "client"), { recursive: true });
await rename(join(source, "worker", "entry.mjs"), join(source, "worker", "index.js"));

const manifest = createManifest({
  artifactId: "astro-fixture",
  framework: { name: "astro", version: "7.4.7" },
  entry: { type: "worker", path: "worker/index.js", export: "default" },
  assets: { directory: "client", base_path: "/_fastcart/assets/" },
  runtime: { required_capabilities: ["fetch", "streams", "web-crypto"] },
  rendering: { mode: "hybrid", streaming: true },
  build: { build_id: "astro-fixture" },
});

const packaged = await packageArtifact({
  sourceDir: source,
  outputDir: output,
  manifest,
});

console.log(`artifact=${packaged.directory}`);
console.log(`digest=${packaged.digest}`);
