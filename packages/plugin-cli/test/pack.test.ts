import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { gunzipSync } from "node:zlib";

import { packPlugin } from "../src/pack.ts";

async function fixture(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "fastcart-plugin-pack-"));
  await writeFile(
    join(root, "plugin.json"),
    JSON.stringify({
      schema_version: "1",
      id: "demo",
      name: "Demo",
      version: "1.0.0",
      api_version: 2,
      runtime: { engine: "workerd", compatibility_date: "2026-11-01" },
      entry: { type: "worker", path: "worker/index.js" },
    }),
  );
  await mkdir(join(root, "worker"), { recursive: true });
  await mkdir(join(root, "node_modules", "greeter"), { recursive: true });
  await writeFile(
    join(root, "node_modules", "greeter", "package.json"),
    JSON.stringify({ name: "greeter", version: "1.0.0", main: "index.js" }),
  );
  await writeFile(
    join(root, "node_modules", "greeter", "index.js"),
    `export const npmGreeting = "hello from npm";\n`,
  );
  await writeFile(
    join(root, "worker", "greet.ts"),
    `export const greeting = "hello from dependency";\n`,
  );
  await writeFile(
    join(root, "worker", "index.ts"),
    `import { greeting } from "./greet.ts";\n` +
      `import { npmGreeting } from "greeter";\n` +
      `export default { async fetch() { return Response.json({ greeting, npmGreeting }); } };\n`,
  );
  return root;
}

function tarNames(gzip: Buffer): string[] {
  const tar = gunzipSync(gzip);
  const names: string[] = [];
  for (let offset = 0; offset + 512 <= tar.length;) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const name = header.subarray(0, 100).toString("utf8").replace(/\0.*$/, "");
    const prefix = header.subarray(345, 500).toString("utf8").replace(/\0.*$/, "");
    const size = Number.parseInt(
      header.subarray(124, 136).toString("ascii").replace(/\0.*$/, "").trim(),
      8,
    );
    names.push(prefix ? `${prefix}/${name}` : name);
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return names;
}

test("bundles TypeScript, copies the manifest, and writes a deterministic tar.gz", async () => {
  const root = await fixture();
  try {
    const outputs = await mkdtemp(join(tmpdir(), "fastcart-plugin-out-"));
    const first = await packPlugin({ root, out: join(outputs, "a") });
    const second = await packPlugin({ root, out: join(outputs, "b") });
    assert.equal(first.manifest.id, "demo");
    assert.equal(first.digest, second.digest);
    const archive = await readFile(first.archive);
    assert.equal(createHash("sha256").update(archive).digest("hex"), first.digest);
    const names = tarNames(archive);
    assert.deepEqual(names, ["plugin.json", "worker/index.js"]);
    assert.ok((await stat(first.archive)).size > 0);
    const tar = gunzipSync(archive);
    const text = tar.toString("utf8");
    assert.match(text, /hello from dependency/);
    assert.match(text, /hello from npm/);
    await rm(outputs, { recursive: true, force: true });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
