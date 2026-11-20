import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { devPlugin, prepareDevSite } from "../src/dev.ts";
import { initPlugin } from "../src/init.ts";

const workerd = new URL("../../../experiments/workerd/bin/workerd", import.meta.url).pathname;

test("prepareDevSite writes the preview harness", async () => {
  const root = await mkdtemp(join(tmpdir(), "fastcart-plugin-dev-"));
  try {
    await initPlugin({ dir: join(root, "demo-plugin"), template: "admin" });
    const plugin = join(root, "demo-plugin");
    const site = await prepareDevSite(plugin, join(plugin, ".fastcart"));
    const config = await readFile(site.config, "utf8");
    assert.match(config, /name = "assets"/);
    assert.match(config, /plugin\.js/);
    const index = await readFile(join(site.out, "assets", "index.html"), "utf8");
    assert.match(index, /demo-plugin/);
    assert.match(index, /settings\.html/);
    const harness = await readFile(join(site.out, "harness.js"), "utf8");
    assert.match(harness, /ASSETS/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("dev serves the worker and preview when workerd is available", async (t) => {
  if (!existsSync(workerd)) {
    t.skip("workerd binary is absent");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "fastcart-plugin-dev-run-"));
  try {
    await initPlugin({ dir: join(root, "demo-plugin"), template: "minimal" });
    const plugin = join(root, "demo-plugin");
    const session = await devPlugin({
      root: plugin,
      out: join(plugin, ".fastcart"),
      serve: true,
      port: 18793,
      workerd,
      intervalMs: 60_000,
    });
    try {
      const home = await fetch(`${session.url}`);
      assert.equal(home.status, 200);
      assert.match(await home.text(), /demo-plugin/);
      const worker = await fetch(`http://127.0.0.1:18793/apps/demo-plugin/`);
      assert.equal(worker.status, 200);
      assert.equal((await worker.json()).message, "Hello from your FastCart plugin");
    } finally {
      await session.stop();
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
