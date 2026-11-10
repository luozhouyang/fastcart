import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { initPlugin } from "../src/init.ts";
import { doctorPlugin } from "../src/doctor.ts";
import { packPlugin } from "../src/pack.ts";

test("init scaffolds each template and pack accepts it", async () => {
  const parent = await mkdtemp(join(tmpdir(), "fastcart-plugin-init-"));
  try {
    for (const template of ["minimal", "admin", "chat"] as const) {
      const dir = join(parent, `${template}-plugin`);
      const result = await initPlugin({ dir, template });
      assert.equal(result.template, template);
      const manifest = JSON.parse(await readFile(join(dir, "plugin.json"), "utf8"));
      assert.equal(manifest.id, `${template}-plugin`);
      const doctor = await doctorPlugin(dir);
      assert.deepEqual(doctor.missing, [], template);
      const packed = await packPlugin({ root: dir, out: join(dir, "dist") });
      assert.ok(packed.files >= 2, template);
    }
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});

test("init refuses a non-empty directory without force", async () => {
  const dir = await mkdtemp(join(tmpdir(), "fastcart-plugin-existing-"));
  try {
    await writeFile(join(dir, "keep.txt"), "x");
    await assert.rejects(() => initPlugin({ dir }), /not empty/);
    await initPlugin({ dir, force: true });
    assert.match(await readFile(join(dir, "plugin.json"), "utf8"), /"id":/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
