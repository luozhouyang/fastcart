// Sidecar bind guard: loopback may run without a secret; anything else may not.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { assertSafeBind, isLoopbackHost } from "../scripts/bind-guard.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SERVE = path.resolve(HERE, "../scripts/serve.mjs");

test("recognizes loopback listen hosts", () => {
  for (const host of ["127.0.0.1", "::1", "[::1]", "localhost", "LOCALHOST", " localhost "]) {
    assert.equal(isLoopbackHost(host), true, `${host} is loopback`);
  }
  for (const host of ["0.0.0.0", "::", "", "127.0.0.2", "sidecar.internal"]) {
    assert.equal(isLoopbackHost(host), false, `${host} is not loopback`);
  }
});

test("accepts a non-loopback bind when a secret is configured", () => {
  assert.doesNotThrow(() => assertSafeBind("0.0.0.0", "test-secret"));
  assert.doesNotThrow(() => assertSafeBind("127.0.0.1", null));
  assert.doesNotThrow(() => assertSafeBind("localhost", ""));
});

test("rejects a non-loopback bind without a secret", () => {
  for (const secret of [null, undefined, "", "   "]) {
    assert.throws(
      () => assertSafeBind("0.0.0.0", secret),
      /SIDECAR_SECRET/,
      `${JSON.stringify(secret)} must not count as a secret`,
    );
  }
});

test("refuses to start on a non-loopback host without a secret", async () => {
  const child = spawn(process.execPath, [SERVE], {
    env: {
      ...process.env,
      HOST: "0.0.0.0",
      PORT: "0",
      SIDECAR_SECRET: "",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });

  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    child.kill("SIGKILL");
  }, 5_000);
  const [code] = await once(child, "close");
  clearTimeout(timer);

  assert.equal(timedOut, false, "sidecar should refuse the bind instead of listening");
  assert.equal(code, 1);
  assert.match(stderr, /refusing to start/);
  assert.match(stderr, /SIDECAR_SECRET/);
});
