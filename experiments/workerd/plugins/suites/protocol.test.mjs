import { test, before, after } from "node:test";
import assert from "node:assert/strict";

import { fixture } from "../harness/commerce.mjs";
import { loadFixture } from "../harness/fixtures.mjs";
import { gate } from "../harness/record.mjs";
import { startSuite } from "../harness/suite.mjs";

let suite;
const ids = {};

before(async () => {
  const ws = loadFixture("ws");
  const stream = loadFixture("stream");
  const cookies = loadFixture("cookies");
  const headers = loadFixture("headers");
  const fixtures = [
    (ids.ws = fixture("ws", ws.modules)),
    (ids.stream = fixture("stream", stream.modules)),
    (ids.cookies = fixture("cookies", cookies.modules)),
    (ids.headers = fixture("headers", headers.modules)),
  ];
  suite = await startSuite({ fixtures });
});

after(async () => {
  await suite?.stop();
});

test("V23 WebSocket upgrade through the loader", async () => {
  await gate("V23", async () => {
    const url = `${suite.base.replace("http", "ws")}/p/ws@${ids.ws.digest}/socket`;
    const socket = new WebSocket(url);
    const messages = [];
    const welcome = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("no welcome within 3s")), 3_000);
      socket.addEventListener("message", (event) => {
        messages.push(event.data);
        if (messages.length === 1) {
          clearTimeout(timer);
          resolve(event.data);
        }
      });
      socket.addEventListener("error", (event) =>
        reject(new Error(`socket error: ${event.message ?? "unknown"}`)),
      );
    });
    assert.equal(welcome, "welcome");
    socket.send("ping");
    const echo = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("no echo within 3s")), 3_000);
      socket.addEventListener("message", (event) => {
        if (String(event.data).startsWith("echo:")) {
          clearTimeout(timer);
          resolve(event.data);
        }
      });
    });
    assert.equal(echo, "echo:ping");
    socket.close();
    return { welcome, echo };
  });
});

test("V24 streaming response preserves timing", async () => {
  await gate("V24", async () => {
    const started = performance.now();
    const response = await fetch(`${suite.base}/p/stream@${ids.stream.digest}/`);
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let text = "";
    let firstChunkMs = null;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (firstChunkMs === null) firstChunkMs = Math.round(performance.now() - started);
      text += decoder.decode(value, { stream: true });
    }
    const totalMs = Math.round(performance.now() - started);
    assert.equal(text, "firstsecond");
    assert.ok(firstChunkMs < 250, `first chunk took ${firstChunkMs}ms`);
    assert.ok(totalMs >= 250, `total took ${totalMs}ms`);
    return { firstChunkMs, totalMs };
  });
});

test("V25 multiple Set-Cookie headers", async () => {
  await gate("V25", async () => {
    const response = await fetch(`${suite.base}/p/cookies@${ids.cookies.digest}/`);
    const cookies = response.headers.getSetCookie();
    assert.equal(cookies.length, 2);
    return { cookies };
  });
});

test("V26 trusted headers and spoofing", async () => {
  await gate("V26", async () => {
    const plain = await (await fetch(`${suite.base}/p/headers@${ids.headers.digest}/`)).json();
    assert.equal(plain.token, "p0-token-headers");
    assert.equal(plain.store, "store_p0");
    assert.equal(plain.plugin, "headers");
    const spoofed = await (
      await fetch(`${suite.base}/p/headers@${ids.headers.digest}/`, {
        headers: { "x-fastcart-store-id": "store_evil" },
      })
    ).json();
    assert.equal(spoofed.store, "store_p0", "gateway must override client-supplied store headers");
    return { plain, spoofed };
  });
});
