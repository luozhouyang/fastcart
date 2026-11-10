import { test, before, after } from "node:test";

import { fixture } from "../harness/commerce.mjs";
import { rssKb } from "../harness/proc.mjs";
import { info } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";

let suite;
const ids = { plugins: [] };

before(async () => {
  ids.plugins = Array.from({ length: 10 }, (_, index) => {
    const source = `let calls = 0;\nexport default { async fetch() { calls += 1; return Response.json({ index: ${index}, calls }); } };`;
    return fixture(`perf-${index}`, { "index.js": source });
  });
  suite = await startSuite({ fixtures: ids.plugins });
});

after(async () => {
  await suite?.stop();
});

test("V40 cold start from spawn to first response", async () => {
  await info("V40", async () => ({ bootMs: Math.round(suite.bootMs * 100) / 100 }));
});

test("V41 warm request latency (500 requests)", async () => {
  await info("V41", async () => {
    const target = `${suite.base}/p/perf-0@${ids.plugins[0].digest}/`;
    for (let index = 0; index < 50; index += 1) await (await fetch(target)).arrayBuffer();
    const samples = [];
    for (let index = 0; index < 500; index += 1) {
      const started = performance.now();
      const response = await fetch(target);
      await response.arrayBuffer();
      samples.push(performance.now() - started);
    }
    samples.sort((a, b) => a - b);
    const pick = (q) => Math.round(samples[Math.floor(samples.length * q)] * 100) / 100;
    return {
      count: samples.length,
      p50: pick(0.5),
      p95: pick(0.95),
      p99: pick(0.99),
      max: Math.round(samples.at(-1) * 100) / 100,
    };
  });
});

test("V42 RSS with 10 loaded plugins", async () => {
  await info("V42", async () => {
    const baselineKb = rssKb(suite.pid);
    for (const plugin of ids.plugins) {
      await fetch(`${suite.base}/p/${plugin.id}@${plugin.digest}/`);
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
    const loadedKb = rssKb(suite.pid);
    return {
      baselineKb,
      loadedKb,
      perPluginKb: baselineKb ? Math.round((loadedKb - baselineKb) / ids.plugins.length) : null,
    };
  });
});
