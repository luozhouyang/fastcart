import { cp, mkdir, rm } from "node:fs/promises";
import { join } from "node:path";

import { createManifest, packageArtifact } from "../../../packages/storefront-build/src/index.ts";

const root = new URL("..", import.meta.url).pathname;
const source = join(root, "dist/artifact");
const output = join(root, "dist/artifacts");

await rm(source, { recursive: true, force: true });
await rm(output, { recursive: true, force: true });
await mkdir(join(source, "worker"), { recursive: true });
await mkdir(join(source, "client"), { recursive: true });
await cp(join(root, "src/worker.js"), join(source, "worker/index.js"));
await cp(join(root, "src/client"), join(source, "client"), { recursive: true });

const manifest = createManifest({
  artifactId: "generic-fixture",
  framework: { name: "generic-fetch-handler", version: "1.0.0" },
  entry: { type: "worker", path: "worker/index.js", export: "default" },
  assets: { directory: "client", base_path: "/_fastcart/assets/" },
  runtime: { required_capabilities: ["fetch", "streams", "web-crypto"] },
  rendering: { mode: "hybrid", streaming: true },
  build: { build_id: "generic-fixture" },
});

const packaged = await packageArtifact({
  sourceDir: source,
  outputDir: output,
  manifest,
});

console.log(`artifact=${packaged.directory}`);
console.log(`digest=${packaged.digest}`);
