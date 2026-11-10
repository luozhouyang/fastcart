import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";

import { PLUGIN_ROOT } from "../harness/env.mjs";
import {
  BODY_MARKER,
  HEAD_MARKER,
  SENTINEL,
  inject,
  injectChunked,
  shouldInject,
} from "../harness/inject.mjs";
import { gate, info } from "../harness/record.mjs";

const corpus = (name) => readFileSync(path.join(PLUGIN_ROOT, "html", name), "utf8");
const headTags = '<link rel="stylesheet" href="/x.css">';
const bodyTags = '<script type="module" src="/runtime.js"></script>';

function count(haystack, needle) {
  return haystack.split(needle).length - 1;
}

test("V31 comment marker injects in place", async () => {
  await gate("V31", async () => {
    const html = corpus("marker-comment.html");
    const out = inject(html, { headTags, bodyTags });
    assert.equal(count(out, HEAD_MARKER), 0);
    assert.equal(count(out, BODY_MARKER), 0);
    assert.equal(count(out, headTags), 1);
    assert.equal(count(out, bodyTags), 1);
    assert.ok(out.indexOf(headTags) < out.indexOf("</head>"));
    assert.ok(out.indexOf(bodyTags) < out.indexOf("</body>"));
    assert.equal(count(out, SENTINEL), 1);
    return { headAt: out.indexOf(headTags), bodyAt: out.indexOf(bodyTags) };
  });
});

test("V32 script marker form injects in place", async () => {
  await gate("V32", async () => {
    const out = inject(corpus("marker-script.html"), { headTags, bodyTags });
    assert.equal(count(out, headTags), 1);
    assert.equal(count(out, bodyTags), 1);
    assert.equal(count(out, "application/fastcart-extensions"), 0);
    return { length: out.length };
  });
});

test("V33 fallback injects before the closing tags", async () => {
  await gate("V33", async () => {
    const out = inject(corpus("no-marker.html"), { headTags, bodyTags });
    assert.equal(count(out, headTags), 1);
    assert.equal(count(out, bodyTags), 1);
    assert.ok(out.indexOf(headTags) < out.indexOf("</head>"));
    assert.ok(out.indexOf(bodyTags) < out.indexOf("</body>"));
    return { headAt: out.indexOf(headTags), bodyAt: out.indexOf(bodyTags) };
  });
});

test("V34 closing tag inside a script string is ignored", async () => {
  await gate("V34", async () => {
    const html = corpus("body-in-script.html");
    const out = inject(html, { headTags, bodyTags });
    const decoy = out.indexOf('"</body>"');
    const injection = out.indexOf(bodyTags);
    assert.ok(decoy !== -1);
    assert.ok(injection > decoy, "must inject before the real closing tag");
    assert.equal(count(out, bodyTags), 1);
    return { decoy, injection };
  });
});

test("V35 every chunk split produces the same output", async () => {
  await gate("V35", async () => {
    const cases = ["marker-comment.html", "no-marker.html"];
    let splits = 0;
    for (const name of cases) {
      const html = corpus(name);
      const expected = inject(html, { headTags, bodyTags });
      for (let index = 0; index <= html.length; index += 1) {
        const actual = injectChunked([html.slice(0, index), html.slice(index)], {
          headTags,
          bodyTags,
        });
        assert.equal(actual, expected, `${name} split at ${index}`);
        splits += 1;
      }
    }
    return { splits };
  });
});

test("V36 skip matrix", async () => {
  await gate("V36", async () => {
    const body = corpus("marker-comment.html");
    const cases = [
      { input: { body }, expected: true },
      { input: { body, status: 404 }, expected: false },
      { input: { body, contentType: "text/plain" }, expected: false },
      { input: { body, contentEncoding: "gzip" }, expected: false },
      { input: { body: corpus("fragment.html") }, expected: false },
      { input: { body: corpus("no-closing-tag.html") }, expected: true },
    ];
    for (const { input, expected } of cases) {
      assert.equal(
        shouldInject(input),
        expected,
        JSON.stringify({
          status: input.status,
          contentType: input.contentType,
          encoding: input.contentEncoding,
        }),
      );
    }
    return { cases: cases.length };
  });
});

test("V37 duplicate markers are deterministic", async () => {
  await gate("V37", async () => {
    const out = inject(corpus("duplicate-marker.html"), { headTags, bodyTags });
    assert.equal(count(out, bodyTags), 1);
    assert.equal(count(out, BODY_MARKER), 1, "the second marker is left untouched");
    return { markersLeft: count(out, BODY_MARKER) };
  });
});

test("V38 injection is idempotent", async () => {
  await gate("V38", async () => {
    const html = corpus("no-marker.html");
    const once = inject(html, { headTags, bodyTags });
    const twice = inject(once, { headTags, bodyTags });
    assert.equal(twice, once);
    assert.equal(count(twice, bodyTags), 1);
    return { length: once.length };
  });
});

test("V39 strict mode without a marker is untouched", async () => {
  await gate("V39", async () => {
    const html = corpus("no-marker.html");
    const out = inject(html, { headTags, bodyTags, strict: true });
    assert.equal(out, html);
    return { length: out.length };
  });
});

test("V43 injector overhead on a 1 MB document", async () => {
  await info("V43", async () => {
    const big = `<!doctype html><html><head><title>big</title></head><body>${"<p>row</p>".repeat(100_000)}</body></html>`;
    const start = performance.now();
    let output = "";
    for (let index = 0; index < 50; index += 1) output = inject(big, { headTags, bodyTags });
    const totalMs = performance.now() - start;
    return {
      bytes: big.length,
      iterations: 50,
      totalMs: Math.round(totalMs),
      perOpMs: Math.round((totalMs / 50) * 100) / 100,
      length: output.length,
    };
  });
});
