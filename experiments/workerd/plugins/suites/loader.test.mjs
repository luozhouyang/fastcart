import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { buildConfig } from "../harness/config.mjs";
import { fixture } from "../harness/commerce.mjs";
import { freePort, HARNESS_DIR } from "../harness/env.mjs";
import { loadFixture } from "../harness/fixtures.mjs";
import { gate, info } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";
import { runWorkerdOnce } from "../harness/workerd.mjs";

let suite;
let v1;
let v2;

before(async () => {
  const hello = loadFixture("hello");
  v1 = fixture("hello", hello.modules, { mainModule: hello.mainModule });
  const v2source = hello.modules["index.js"].replace(
    'return new Response("hello");',
    'return new Response("hello-v2");',
  );
  v2 = fixture(
    "hello",
    { ...hello.modules, "index.js": `${v2source}\n// v2` },
    { mainModule: hello.mainModule },
  );
  suite = await startSuite({ fixtures: [v1, v2] });
});

after(async () => {
  await suite?.stop();
});

test("V01 loader config requires --experimental", async () => {
  await gate("V01", async () => {
    const dir = await mkdtemp(path.join(os.tmpdir(), "fc-p0-v01-"));
    try {
      await writeFile(
        path.join(dir, "gateway.js"),
        await readFile(path.join(HARNESS_DIR, "gateway.js")),
      );
      await writeFile(
        path.join(dir, "bridge.js"),
        await readFile(path.join(HARNESS_DIR, "bridge.js")),
      );
      const port = await freePort();
      await writeFile(path.join(dir, "config.capnp"), buildConfig({ port, commercePort: 1 }));
      const result = await runWorkerdOnce({ configPath: path.join(dir, "config.capnp"), cwd: dir });
      assert.notEqual(
        result.code,
        0,
        "workerd should refuse the loader config without --experimental",
      );
      assert.match(result.output, /experimental/i);
      return { exitCode: result.code };
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });
});

test("V02 loads a plugin and serves a request", async () => {
  await gate("V02", async () => {
    const response = await fetch(`${suite.base}/p/hello@${v1.digest}/`);
    assert.equal(response.status, 200);
    assert.equal(await response.text(), "hello");
    return { status: response.status };
  });
});

test("V03 loader id caches the worker instance", async () => {
  await gate("V03", async () => {
    const first = await (await fetch(`${suite.base}/p/hello@${v1.digest}/state`)).json();
    const second = await (await fetch(`${suite.base}/p/hello@${v1.digest}/state`)).json();
    assert.equal(second.calls, first.calls + 1);
    return { first: first.calls, second: second.calls };
  });
});

test("V04 a new digest loads a fresh instance", async () => {
  await gate("V04", async () => {
    const before = await (await fetch(`${suite.base}/p/hello@${v1.digest}/state`)).json();
    const v2body = await (await fetch(`${suite.base}/p/hello@${v2.digest}/`)).text();
    const after = await (await fetch(`${suite.base}/p/hello@${v1.digest}/state`)).json();
    assert.equal(v2body, "hello-v2");
    assert.equal(after.calls, before.calls + 1, "v1 state must survive loading v2");
    return { before: before.calls, after: after.calls };
  });
});

test("V05 a missing bundle fails cleanly and the runtime stays healthy", async () => {
  await gate("V05", async () => {
    const missing = await fetch(`${suite.base}/p/missing@deadbeef/`);
    assert.equal(missing.status, 502);
    const body = await missing.json();
    assert.match(body.error, /unavailable|not found/i);
    const healthy = await fetch(`${suite.base}/p/hello@${v1.digest}/`);
    assert.equal(healthy.status, 200);
    return { missingStatus: missing.status, error: body.error };
  });
});

test("V40a cold start (suite boot) recorded", async () => {
  await info("V40a", async () => ({ bootMs: Math.round(suite.bootMs * 100) / 100 }));
});
