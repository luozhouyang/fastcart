import { test, before, after } from "node:test";
import assert from "node:assert/strict";

import { fixture } from "../harness/commerce.mjs";
import { loadFixture } from "../harness/fixtures.mjs";
import { gate, info } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";

let suite;
const ids = {};

before(async () => {
  const allow = loadFixture("node-allow");
  const deny = loadFixture("node-deny");
  const fixtures = [
    (ids.allow = fixture("node-allow", allow.modules, { flags: ["nodejs_compat"] })),
    (ids.deny = fixture("node-deny", deny.modules, { flags: ["nodejs_compat"] })),
  ];
  suite = await startSuite({ fixtures });
});

after(async () => {
  await suite?.stop();
});

test("V27 nodejs_compat allowlist", async () => {
  await gate("V27", async () => {
    const response = await fetch(`${suite.base}/p/node-allow@${ids.allow.digest}/`);
    assert.equal(response.status, 200, await response.clone().text());
    const body = await response.json();
    assert.equal(body.buffer, "aGk=");
    assert.equal(body.processType, "object");
    assert.equal(body.joined, "a/b");
    assert.equal(body.utilFormat, "function");
    assert.equal(body.uuidLength, 36);
    assert.equal(body.readable, "function");
    assert.equal(body.events, "function");
    return body;
  });
});

test("V28 nodejs_compat denylist", async () => {
  await gate("V28", async () => {
    const response = await fetch(`${suite.base}/p/node-deny@${ids.deny.digest}/`);
    assert.equal(response.status, 200, await response.clone().text());
    const body = await response.json();
    // node:fs exists but is a virtual filesystem: host paths must not be readable.
    assert.equal(body["node:fs"].imported, true);
    assert.equal(body["node:fs"].read.ok, false, "host files must not be readable");
    const listed = String(body["node:fs"].list.value ?? "")
      .split(",")
      .filter(Boolean);
    assert.ok(
      listed.every((name) => ["bundle", "tmp", "dev"].includes(name)),
      `unexpected fs entries: ${listed}`,
    );
    // node:net may import, but connect must not work without an outbound binding.
    assert.equal(body["node:net"].connect?.ok ?? false, false, "net.connect must be blocked");
    // child_process may exist, but it must not execute anything.
    const child = body["node:child_process"] ?? {};
    assert.equal(child.exec?.ok ?? false, false, "execSync must fail");
    assert.equal(child.spawn?.ok ?? false, false, "spawnSync must fail");
    return body;
  });
});

test("V29 process.env exposure", async () => {
  await gate("V29", async () => {
    const body = await (await fetch(`${suite.base}/p/node-allow@${ids.allow.digest}/`)).json();
    const keys = body.envKeys ?? [];
    assert.ok(!keys.includes("PATH"), "host PATH must not leak");
    assert.ok(!keys.includes("HOME"), "host HOME must not leak");
    return { envKeys: keys };
  });
});

test("V30 module availability without the flag", async () => {
  await info("V30", async () => {
    const allow = loadFixture("node-allow");
    const plain = fixture("node-plain", allow.modules);
    const plainSuite = await startSuite({ fixtures: [plain] });
    try {
      const response = await fetch(`${plainSuite.base}/p/node-plain@${plain.digest}/`);
      return { status: response.status, body: (await response.text()).slice(0, 160) };
    } finally {
      await plainSuite.stop();
    }
  });
});
