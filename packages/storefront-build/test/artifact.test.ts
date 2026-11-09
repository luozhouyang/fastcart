import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import {
  ArtifactBuildError,
  computeArtifactDigest,
  createManifest,
  packageArtifact,
  readManifest,
} from "../src/index.ts";
import type { ArtifactManifest } from "../src/index.ts";

async function fixture(): Promise<{ root: string; manifest: ArtifactManifest }> {
  const root = await mkdtemp(join(tmpdir(), "fastcart-artifact-"));
  await mkdir(join(root, "worker"), { recursive: true });
  await mkdir(join(root, "client/assets"), { recursive: true });
  await writeFile(
    join(root, "worker/index.js"),
    "export default { fetch() { return new Response('ok') } }\n",
  );
  await writeFile(join(root, "client/assets/app.js"), "console.log(1)\n");
  const manifest = createManifest({
    artifactId: "artifact_01",
    framework: { name: "tanstack-start", version: "1.169.5" },
    entry: { type: "worker", path: "worker/index.js", export: "default" },
    assets: { directory: "client", base_path: "/_fastcart/assets/" },
    runtime: { required_capabilities: ["fetch"] },
    build: { build_id: "build_123", created_at: "2026-11-01T10:00:00Z" },
  });
  return { root, manifest };
}

test("packages an artifact by digest and reads it back", async () => {
  const { root, manifest } = await fixture();
  const output = await mkdtemp(join(tmpdir(), "fastcart-output-"));
  try {
    const packaged = await packageArtifact({
      sourceDir: root,
      outputDir: output,
      manifest,
    });
    assert.equal(packaged.digest.length, 64);
    assert.equal(packaged.directory, join(output, packaged.digest));
    const read = await readManifest(packaged.directory);
    assert.equal(read.artifact_id, "artifact_01");
    assert.equal(read.entry.path, "worker/index.js");
    assert.equal(await computeArtifactDigest(packaged.directory), packaged.digest);

    const again = await packageArtifact({
      sourceDir: root,
      outputDir: output,
      manifest,
    });
    assert.equal(again.digest, packaged.digest);
  } finally {
    await rm(root, { recursive: true, force: true });
    await rm(output, { recursive: true, force: true });
  }
});

test("digest matches the Rust artifact algorithm", async () => {
  const root = await mkdtemp(join(tmpdir(), "fastcart-digest-"));
  try {
    await writeFile(join(root, "a.js"), "a");
    await mkdir(join(root, "dir"));
    await writeFile(join(root, "dir/b.js"), "b");
    assert.equal(
      await computeArtifactDigest(root),
      "2f7c7d75e1f26542aa87b4547891d398475450e46cc7186c40de981b12e0e10b",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects traversal and missing entries", async () => {
  const { root, manifest } = await fixture();
  const output = await mkdtemp(join(tmpdir(), "fastcart-output-"));
  try {
    await assert.rejects(
      () =>
        packageArtifact({
          sourceDir: root,
          outputDir: output,
          manifest: { ...manifest, entry: { ...manifest.entry, path: "../escape.js" } },
        }),
      ArtifactBuildError,
    );
    await rm(join(root, "worker/index.js"));
    await assert.rejects(
      () => packageArtifact({ sourceDir: root, outputDir: output, manifest }),
      ArtifactBuildError,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
    await rm(output, { recursive: true, force: true });
  }
});

test("rejects unsupported capabilities", () => {
  assert.throws(
    () =>
      createManifest({
        artifactId: "artifact_01",
        framework: { name: "astro", version: "7.4.7" },
        entry: { type: "worker", path: "worker/index.js", export: "default" },
        assets: { directory: "client", base_path: "/_fastcart/assets/" },
        runtime: {
          required_capabilities: ["durable-objects" as never],
        },
      }),
    ArtifactBuildError,
  );
});
