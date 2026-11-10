import { test, before, after } from "node:test";
import { setTimeout as delay } from "node:timers/promises";

import { fixture } from "../harness/commerce.mjs";
import { loadFixture } from "../harness/fixtures.mjs";
import { rssKb } from "../harness/proc.mjs";
import { gate, info } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";

let suite;
const ids = { variants: [] };

before(async () => {
  const concurrency = loadFixture("concurrency");
  const hello = loadFixture("hello");
  const loop = loadFixture("loop-sync");
  const slow = loadFixture("slow");
  const fixtures = [
    (ids.concurrency = fixture("concurrency", concurrency.modules)),
    (ids.hello = fixture("hello", hello.modules)),
    (ids.loop = fixture("loop-sync", loop.modules)),
    (ids.slow = fixture("slow", slow.modules)),
  ];
  for (let index = 0; index < 30; index += 1) {
    const source = `${hello.modules["index.js"]}\n// variant ${index}`;
    ids.variants.push(fixture(`mem-${index}`, { "index.js": source }));
  }
  suite = await startSuite({ fixtures: [...fixtures, ...ids.variants] });
});

after(async () => {
  await suite?.stop();
});

test("V18 concurrent calls to one loader id", async () => {
  await gate("V18", async () => {
    await fetch(`${suite.base}/p/concurrency@${ids.concurrency.digest}/reset`);
    const started = performance.now();
    const responses = await Promise.all(
      Array.from({ length: 20 }, () =>
        fetch(`${suite.base}/p/concurrency@${ids.concurrency.digest}/serial`).then((r) => r.json()),
      ),
    );
    const elapsedMs = Math.round(performance.now() - started);
    const maxActive = Math.max(...responses.map((item) => item.maxActive));
    return { maxActive, elapsedMs };
  });
});

test("V19 cross-plugin isolation while one plugin spins", async () => {
  await info("V19", async () => {
    const blocker = fetch(`${suite.base}/p/loop-sync@${ids.loop.digest}/loop?ms=3000`).catch(
      (error) => String(error),
    );
    await delay(100);
    const started = performance.now();
    let healthy;
    try {
      const response = await fetch(`${suite.base}/p/hello@${ids.hello.digest}/`, {
        signal: AbortSignal.timeout(3_000),
      });
      healthy = {
        status: response.status,
        body: await response.text(),
        elapsedMs: Math.round(performance.now() - started),
      };
    } catch (error) {
      healthy = { error: String(error), elapsedMs: Math.round(performance.now() - started) };
    }
    await blocker;
    return healthy;
  });
});

test("V20 dispose() exposure", async () => {
  await info("V20", async () => ({
    note: "not exposed by the gateway stub; check manually in P1",
  }));
});

test("V21 repeated load/unload memory", async () => {
  await info("V21", async () => {
    const beforeKb = rssKb(suite.pid);
    for (const variant of ids.variants) {
      await fetch(`${suite.base}/p/${variant.id}@${variant.digest}/`);
    }
    await delay(500);
    const afterKb = rssKb(suite.pid);
    return { beforeKb, afterKb, growthKb: beforeKb && afterKb ? afterKb - beforeKb : null };
  });
});

test("V22 in-flight request when workerd is killed", async () => {
  await info("V22", async () => {
    const pending = fetch(`${suite.base}/p/slow@${ids.slow.digest}/`).then(
      (response) => ({ status: response.status }),
      (error) => ({ error: String(error) }),
    );
    await delay(200);
    suite.child.kill("SIGKILL");
    const result = await Promise.race([pending, delay(3_000).then(() => ({ timeout: true }))]);
    return result;
  });
});
