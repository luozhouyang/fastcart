import { test, before, after } from "node:test";

import { fixture } from "../harness/commerce.mjs";
import { loadFixture } from "../harness/fixtures.mjs";
import { info } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";

let suite;
const ids = {};

before(async () => {
  const sync = loadFixture("loop-sync");
  const asyncLoop = loadFixture("loop-async");
  const subs = loadFixture("subrequests");
  const fixtures = [
    (ids.sync = fixture("loop-sync", sync.modules, { limits: { cpuMs: 100 } })),
    (ids.entry = fixture("loop-entry", sync.modules, { entryLimits: { cpuMs: 50 } })),
    (ids.async = fixture("loop-async", asyncLoop.modules, { limits: { cpuMs: 100 } })),
    (ids.subs = fixture("subrequests", subs.modules, { limits: { subRequests: 5 } })),
  ];
  suite = await startSuite({ fixtures });
});

after(async () => {
  await suite?.stop();
});

async function timed(path) {
  const started = performance.now();
  try {
    const response = await fetch(`${suite.base}${path}`, { signal: AbortSignal.timeout(6_000) });
    const body = await response.text();
    return { status: response.status, elapsedMs: Math.round(performance.now() - started), body };
  } catch (error) {
    return {
      status: null,
      elapsedMs: Math.round(performance.now() - started),
      error: String(error),
    };
  }
}

test("V10 cpuMs on a synchronous loop", async () => {
  await info("V10", async () => timed(`/p/loop-sync@${ids.sync.digest}/loop?ms=300`), {
    limit: "cpuMs=100",
  });
});

test("V11 cpuMs on an async loop", async () => {
  await info("V11", async () => timed(`/p/loop-async@${ids.async.digest}/loop`), {
    limit: "cpuMs=100",
  });
});

test("V12 entrypoint-level limits", async () => {
  await info("V12", async () => timed(`/p/loop-entry@${ids.entry.digest}/loop?ms=300`), {
    limit: "entry cpuMs=50",
  });
});

test("V13 subRequests limit", async () => {
  await info("V13", async () => timed(`/p/subrequests@${ids.subs.digest}/sub?n=8`), {
    limit: "subRequests=5",
  });
});
