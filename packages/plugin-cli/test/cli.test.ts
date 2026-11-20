import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { promisify } from "node:util";

const run = promisify(execFile);

test("cli init then pack prints an archive and digest", async () => {
  const parent = await mkdtemp(join(tmpdir(), "fastcart-plugin-cli-"));
  const dir = join(parent, "cli-plugin");
  try {
    const init = await run(
      process.execPath,
      ["--experimental-strip-types", "src/cli.ts", "init", dir, "--template", "minimal"],
      { cwd: new URL("..", import.meta.url).pathname },
    );
    assert.match(init.stdout, /Created minimal plugin/);
    const pack = await run(
      process.execPath,
      ["--experimental-strip-types", "src/cli.ts", "pack", dir, "--out", join(parent, "out")],
      { cwd: new URL("..", import.meta.url).pathname },
    );
    assert.match(pack.stdout, /Packed cli-plugin@0\.1\.0/);
    assert.match(pack.stdout, /sha256:/);
    const manifest = JSON.parse(await readFile(join(dir, "plugin.json"), "utf8"));
    assert.equal(manifest.id, "cli-plugin");
  } finally {
    await rm(parent, { recursive: true, force: true });
  }
});
