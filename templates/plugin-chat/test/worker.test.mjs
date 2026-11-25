import assert from "node:assert/strict";
import { test } from "node:test";

import worker from "../worker/index.js";

const ORIGIN = "https://shop.example.com";

function call(path, init = {}, headers = {}) {
  const request = new Request(`${ORIGIN}${path}`, {
    ...init,
    headers: { ...(init.headers ?? {}), ...headers },
  });
  return worker.fetch(request, { FASTCART_PLUGIN_ID: "chat-widget" });
}

function withSettings(settings, calls = []) {
  return {
    FASTCART_PLUGIN_ID: "chat-widget",
    FASTCART_API: {
      async fetch(url, init) {
        calls.push({ url: String(url), init });
        return Response.json({ settings });
      },
    },
  };
}

test("session returns settings from the host context", async () => {
  const calls = [];
  const response = await worker.fetch(
    new Request(`${ORIGIN}/capabilities/livechat/session`, {
      headers: {
        "x-fastcart-store-id": "store_1",
        "x-fastcart-context-token": "token-1",
      },
    }),
    withSettings({ greeting: "Hello from settings", accent: "#111827" }, calls),
  );
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.deepEqual(body, {
    capability: "livechat",
    version: 1,
    greeting: "Hello from settings",
    accent: "#111827",
    context: { storeId: "store_1" },
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "http://api/internal/plugins/v1/context");
  assert.equal(calls[0].init.headers.authorization, "Bearer token-1");
  assert.equal(JSON.stringify(body).includes("token-1"), false);
});

test("session falls back to defaults without a context token", async () => {
  const response = await call(
    "/capabilities/livechat/session",
    {},
    {
      "x-fastcart-store-id": "store_2",
    },
  );
  assert.equal(response.status, 200);
  const body = await response.json();
  assert.equal(body.greeting, "Hi! How can we help?");
  assert.equal(body.accent, "#2563eb");
  assert.equal(body.context.storeId, "store_2");
});

test("session defaults missing or empty settings values", async () => {
  const calls = [];
  const response = await worker.fetch(
    new Request(`${ORIGIN}/capabilities/livechat/session?store=spoofed`, {
      headers: {
        "x-fastcart-store-id": "store_3",
        "x-fastcart-context-token": "token-3",
      },
    }),
    withSettings({ greeting: " ", accent: "" }, calls),
  );
  const body = await response.json();
  assert.equal(body.greeting, "Hi! How can we help?");
  assert.equal(body.accent, "#2563eb");
  assert.equal(body.context.storeId, "store_3", "only the trusted header decides the store");
});

test("capability root lists its endpoints", async () => {
  const response = await call("/capabilities/livechat");
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), {
    capability: "livechat",
    version: 1,
    endpoints: {
      session: "/capabilities/livechat/session",
      messages: "/capabilities/livechat/messages",
    },
  });
});

test("messages echoes an accepted message", async () => {
  const response = await call(
    "/capabilities/livechat/messages",
    {
      method: "POST",
      body: JSON.stringify({ message: "  Where is my order?  " }),
    },
    { "x-fastcart-store-id": "store_1", "content-type": "application/json" },
  );
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), {
    capability: "livechat",
    ok: true,
    echo: "Where is my order?",
    context: { storeId: "store_1" },
  });
});

test("messages rejects non-string and empty messages", async () => {
  const nonString = await call("/capabilities/livechat/messages", {
    method: "POST",
    body: JSON.stringify({ message: 42 }),
  });
  assert.equal(nonString.status, 400);
  assert.equal((await nonString.json()).error, "message must be a string");

  const empty = await call("/capabilities/livechat/messages", {
    method: "POST",
    body: JSON.stringify({ message: "   " }),
  });
  assert.equal(empty.status, 400);
  assert.equal((await empty.json()).error, "message must not be empty");
});

test("messages rejects an oversized message", async () => {
  const response = await call("/capabilities/livechat/messages", {
    method: "POST",
    body: JSON.stringify({ message: "a".repeat(2001) }),
  });
  assert.equal(response.status, 400);
  assert.match((await response.json()).error, /at most 2000/);
});

test("messages rejects bodies over the size limit", async () => {
  const declared = await call(
    "/capabilities/livechat/messages",
    {
      method: "POST",
      headers: { "content-length": "9000" },
      body: "{}",
    },
    {},
  );
  assert.equal(declared.status, 413);

  const actual = await call("/capabilities/livechat/messages", {
    method: "POST",
    body: JSON.stringify({ message: "a".repeat(9000) }),
  });
  assert.equal(actual.status, 413);
});

test("messages rejects invalid JSON and invalid methods", async () => {
  const invalid = await call("/capabilities/livechat/messages", {
    method: "POST",
    body: "not json",
  });
  assert.equal(invalid.status, 400);
  assert.equal((await invalid.json()).error, "body must be JSON");

  const wrongMethod = await call("/capabilities/livechat/messages");
  assert.equal(wrongMethod.status, 405);
  assert.equal(wrongMethod.headers.get("allow"), "POST");
});

test("unknown capability paths return 404", async () => {
  const response = await call("/capabilities/livechat/unknown");
  assert.equal(response.status, 404);
});

test("existing admin and api routes keep working", async () => {
  const admin = await call("/admin/conversations.list", {}, {});
  assert.equal(admin.status, 200);
  const adminBody = await admin.json();
  assert.equal(adminBody.plugin, "chat-widget");
  assert.equal(adminBody.conversations.length, 2);

  const api = await call("/api/hello", { method: "POST", body: "{}" });
  assert.equal(api.status, 200);
  assert.deepEqual(await api.json(), { ok: true, path: "/api/hello" });
});
