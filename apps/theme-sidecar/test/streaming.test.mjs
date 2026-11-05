// Sidecar render protocol tests: buffered, streamed and timeout paths.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SERVE = path.resolve(HERE, "../scripts/serve.mjs");

const THEMES = {
  stream: `
export async function render() {
  return {
    status: 200,
    headers: [
      ["content-type", "text/html; charset=utf-8"],
      ["x-fixture", "stream"],
    ],
    body: (async function* () {
      yield "<!doctype html><html><body>";
      await new Promise((resolve) => setTimeout(resolve, 10));
      yield "<h1>streamed</h1></body></html>";
    })(),
  };
}
`,
  full: `
export async function render() {
  return {
    status: 201,
    headers: [["content-type", "text/html; charset=utf-8"]],
    body: "<h1>buffered</h1>",
  };
}
`,
  broken: `
export async function render() {
  return {
    status: 200,
    headers: [["content-type", "text/html; charset=utf-8"]],
    body: (async function* () {
      yield "<html><body>";
      throw new Error("failed after the shell");
    })(),
  };
}
`,
  slow: `
export async function render() {
  await new Promise(() => {});
  return { status: 200, headers: [], body: "unreachable" };
}
`,
};

let root;
let sidecar;

before(async () => {
  root = await mkdtemp(path.join(tmpdir(), "fastcart-sidecar-"));
  for (const [name, source] of Object.entries(THEMES)) {
    const bundle = path.join(root, name);
    await mkdir(bundle, { recursive: true });
    await writeFile(path.join(bundle, "theme.json"), JSON.stringify({ id: name }));
    await writeFile(path.join(bundle, "server.mjs"), source);
  }
  sidecar = await startSidecar(root);
});

after(async () => {
  if (sidecar) {
    sidecar.child.kill("SIGTERM");
    await Promise.race([
      once(sidecar.child, "exit"),
      new Promise((resolve) => setTimeout(resolve, 1_000)),
    ]);
  }
  if (root) {
    await rm(root, { recursive: true, force: true });
  }
});

test("streams raw bytes with theme headers", async () => {
  const response = await render("stream");
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("x-fastcart-mode"), "stream");
  assert.equal(response.headers.get("x-fixture"), "stream");
  assert.match(response.headers.get("content-type") ?? "", /text\/html/);
  assert.equal(response.headers.get("content-length"), null);
  assert.equal(await response.text(), "<!doctype html><html><body><h1>streamed</h1></body></html>");
});

test("keeps the buffered JSON contract for non-streaming themes", async () => {
  const response = await render("full");
  assert.equal(response.status, 200);
  assert.match(response.headers.get("content-type") ?? "", /application\/json/);
  assert.equal(response.headers.get("x-fastcart-mode"), null);
  const result = await response.json();
  assert.equal(result.status, 201);
  assert.equal(Buffer.from(result.body, "base64").toString("utf8"), "<h1>buffered</h1>");
});

test("buffers a streaming theme when the host does not accept streams", async () => {
  const response = await render("stream", { streaming: false });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("x-fastcart-mode"), null);
  const result = await response.json();
  assert.equal(
    Buffer.from(result.body, "base64").toString("utf8"),
    "<!doctype html><html><body><h1>streamed</h1></body></html>",
  );
});

test("aborts a stream that fails after its headers", async () => {
  const response = await render("broken");
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("x-fastcart-mode"), "stream");
  await assert.rejects(async () => {
    await response.text();
  });
});

test("times out a runaway streaming theme", async () => {
  const started = Date.now();
  const response = await render("slow", { timeout_ms: 100, stream_timeout_ms: 100 });
  assert.equal(response.status, 504);
  const result = await response.json();
  assert.equal(result.error.code, "RENDER_TIMEOUT");
  assert.ok(Date.now() - started < 5_000);

  const health = await fetch(`${sidecar.url}/health`);
  assert.equal(health.status, 200);
});

/** Sends one render request for a fixture bundle. */
async function render(bundle, overrides = {}) {
  const response = await fetch(`${sidecar.url}/render`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      request_id: "req_test",
      store: {
        id: "store_1",
        domain: "shop.example.com",
        locale: "en",
        currency: "USD",
        timezone: "UTC",
      },
      method: "GET",
      url: "/",
      headers: [],
      body: null,
      theme: {
        id: bundle,
        version: "1.0.0",
        runtime: "node",
        bundle_path: bundle,
        entry: "server.mjs",
        settings: {},
        assets_manifest: null,
        assets_base_url: null,
      },
      page: null,
      data: null,
      capabilities: {},
      mode: "production",
      timeout_ms: 1_000,
      streaming: true,
      stream_timeout_ms: 5_000,
      ...overrides,
    }),
  });
  return response;
}

/** Starts the sidecar on an ephemeral port and waits for its listen line. */
function startSidecar(themeRoot) {
  const child = spawn(process.execPath, [SERVE], {
    env: {
      ...process.env,
      PORT: "0",
      HOST: "127.0.0.1",
      THEME_ROOT: themeRoot,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  return new Promise((resolve, reject) => {
    const lines = createInterface({ input: child.stdout });
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error("sidecar did not start"));
    }, 5_000);
    lines.on("line", (line) => {
      const match = /listening on http:\/\/[^:]+:(\d+)/.exec(line);
      if (!match) {
        return;
      }
      clearTimeout(timer);
      lines.close();
      resolve({ child, url: `http://127.0.0.1:${match[1]}` });
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`sidecar exited (${code})`));
    });
  });
}
