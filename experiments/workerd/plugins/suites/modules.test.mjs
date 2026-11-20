import { test, before, after } from "node:test";
import assert from "node:assert/strict";

import { fixture } from "../harness/commerce.mjs";
import { loadFixture } from "../harness/fixtures.mjs";
import { gate, info } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";

let suite;
const ids = {};

function largeGraph(modules, bytesPerModule) {
  const graph = {};
  const imports = [];
  for (let index = 0; index < modules; index += 1) {
    graph[`m${index}.js`] = largeSource(bytesPerModule);
    imports.push(`import "./m${index}.js";`);
  }
  graph["index.js"] =
    `${imports.join("\n")}\nexport default { async fetch() { return Response.json({ modules: ${modules} }); } };\n`;
  return graph;
}

function largeSource(bytes) {
  const lines = [];
  let size = 0;
  let index = 0;
  while (size < bytes) {
    const line = `const value${index} = ${index};\n`;
    lines.push(line);
    size += line.length;
    index += 1;
  }
  lines.push(
    `export default { async fetch() { return Response.json({ size: ${bytes}, statements: ${index} }); } };\n`,
  );
  return lines.join("");
}

before(async () => {
  const modules = loadFixture("modules");
  const types = loadFixture("module-types");
  const fixtures = [
    (ids.modules = fixture("modules", modules.modules, { mainModule: modules.mainModule })),
    (ids.types = fixture("module-types", types.modules, { mainModule: types.mainModule })),
    (ids.large1 = fixture("large1", { "index.js": largeSource(1024 * 1024) })),
    (ids.large8 = fixture("large8", largeGraph(8, 1024 * 1024))),
    (ids.largeSingle = fixture("large-single", { "index.js": largeSource(8 * 1024 * 1024) })),
  ];
  suite = await startSuite({ fixtures });
});

after(async () => {
  await suite?.stop();
});

test("V14 relative imports resolve", async () => {
  await gate("V14", async () => {
    const body = await (await fetch(`${suite.base}/p/modules@${ids.modules.digest}/`)).json();
    assert.equal(body.value, "util-v1");
    return body;
  });
});

test("V15 json and text module types", async () => {
  await info("V15", async () => {
    const response = await fetch(`${suite.base}/p/module-types@${ids.types.digest}/`);
    const text = await response.text();
    return { status: response.status, body: text.slice(0, 200) };
  });
});

test("V16 large module graph", async () => {
  await info("V16", async () => {
    const started = performance.now();
    const response = await fetch(`${suite.base}/p/large8@${ids.large8.digest}/`, {
      signal: AbortSignal.timeout(30_000),
    });
    const body = await response.text();
    return {
      bytes: 8 * 1024 * 1024,
      status: response.status,
      elapsedMs: Math.round(performance.now() - started),
      body: body.slice(0, 120),
    };
  });
});

test("V16b a single 8 MB module", async () => {
  await info("V16b", async () => {
    const started = performance.now();
    const response = await fetch(`${suite.base}/p/large-single@${ids.largeSingle.digest}/`, {
      signal: AbortSignal.timeout(30_000),
    });
    const body = await response.text();
    return {
      bytes: 8 * 1024 * 1024,
      status: response.status,
      elapsedMs: Math.round(performance.now() - started),
      body: body.slice(0, 200),
    };
  });
});

test("V17 first-compile latency for a 1 MB graph", async () => {
  await info("V17", async () => {
    const started = performance.now();
    const response = await fetch(`${suite.base}/p/large1@${ids.large1.digest}/`, {
      signal: AbortSignal.timeout(30_000),
    });
    const body = await response.text();
    return {
      bytes: 1024 * 1024,
      status: response.status,
      compileMs: Math.round(performance.now() - started),
      body,
    };
  });
});
