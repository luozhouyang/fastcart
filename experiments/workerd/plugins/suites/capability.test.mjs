import { test, before, after } from "node:test";
import assert from "node:assert/strict";

import { fixture } from "../harness/commerce.mjs";
import { loadFixture } from "../harness/fixtures.mjs";
import { gate } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";

let suite;
let capability;
let egress;

before(async () => {
  const cap = loadFixture("capability");
  const ego = loadFixture("egress");
  capability = fixture("capability", cap.modules);
  egress = fixture("egress", ego.modules);
  suite = await startSuite({ fixtures: [capability, egress] });
});

after(async () => {
  await suite?.stop();
});

test("V06 env service binding reaches the host API", async () => {
  await gate("V06", async () => {
    const response = await fetch(`${suite.base}/p/capability@${capability.digest}/`);
    assert.equal(response.status, 200);
    const body = await response.json();
    assert.equal(body.plugin, "capability");
    assert.equal(body.upstream.store, "store_p0");
    const call = suite.commerce.requests.find(
      (item) => item.path === "/internal/plugins/v1/context",
    );
    assert.ok(call, "host API should have been called");
    assert.equal(call.headers["x-fastcart-bridge"], "1");
    assert.equal(call.headers.cookie, undefined, "bridge must strip cookies");
    assert.equal(
      call.headers["x-fastcart-store-id"],
      undefined,
      "bridge must strip client store headers",
    );
    return { envKeys: body.envKeys, upstream: body.upstream };
  });
});

test("V07 no ambient bindings are visible", async () => {
  await gate("V07", async () => {
    const body = await (await fetch(`${suite.base}/p/capability@${capability.digest}/`)).json();
    assert.deepEqual(body.envKeys, ["FASTCART_API", "FASTCART_PLUGIN_ID"]);
    return { envKeys: body.envKeys };
  });
});

test("V08 global fetch is blocked", async () => {
  await gate("V08", async () => {
    const body = await (await fetch(`${suite.base}/p/egress@${egress.digest}/fetch`)).json();
    assert.equal(body.blocked, true);
    assert.match(body.error, /internet|not permitted|fetch/i);
    return { error: body.error };
  });
});

test("V09 outbound WebSocket is blocked", async () => {
  await gate("V09", async () => {
    const body = await (await fetch(`${suite.base}/p/egress@${egress.digest}/ws`)).json();
    assert.equal(body.blocked, true, JSON.stringify(body));
    return { error: body.error };
  });
});
