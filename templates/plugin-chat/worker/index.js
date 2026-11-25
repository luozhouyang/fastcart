// Live chat plugin worker.
//
// Admin RPC keeps its existing shape. Storefront capability requests arrive
// under /capabilities/livechat: the host forwards /store/capabilities/livechat
// and /store/capabilities/livechat/{*path} to this prefix with a short-lived
// storefront context token. The host strips client-supplied x-fastcart-*
// headers and reinjects the trusted values, so the request headers below are
// the only source of store identity.

const CAPABILITY_ID = "livechat";
const CAPABILITY_VERSION = 1;
const MAX_MESSAGE_CHARS = 2000;
const MAX_BODY_BYTES = 8 * 1024;
const DEFAULT_GREETING = "Hi! How can we help?";
const DEFAULT_ACCENT = "#2563eb";

function json(payload, status = 200, headers = {}) {
  return Response.json(payload, { status, headers });
}

function methodNotAllowed(allowed) {
  return json({ error: "method not allowed" }, 405, { allow: allowed.join(", ") });
}

function trustedStoreId(request) {
  // The host strips client-supplied x-fastcart-* headers before forwarding,
  // then injects the store resolved from the request Host.
  return request.headers.get("x-fastcart-store-id");
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

async function loadSettings(request, env) {
  const token = request.headers.get("x-fastcart-context-token");
  if (!token || !env || !env.FASTCART_API || typeof env.FASTCART_API.fetch !== "function") {
    return {};
  }
  try {
    const response = await env.FASTCART_API.fetch("http://api/internal/plugins/v1/context", {
      headers: { authorization: `Bearer ${token}`, accept: "application/json" },
    });
    if (!response.ok) return {};
    const payload = await response.json();
    const settings = payload && payload.settings;
    return settings && typeof settings === "object" && !Array.isArray(settings) ? settings : {};
  } catch {
    return {};
  }
}

async function session(request, env) {
  const settings = await loadSettings(request, env);
  return json({
    capability: CAPABILITY_ID,
    version: CAPABILITY_VERSION,
    greeting: nonEmptyString(settings.greeting) ?? DEFAULT_GREETING,
    accent: nonEmptyString(settings.accent) ?? DEFAULT_ACCENT,
    context: { storeId: trustedStoreId(request) },
  });
}

async function messages(request) {
  const declared = Number(request.headers.get("content-length") ?? "0");
  if (Number.isFinite(declared) && declared > MAX_BODY_BYTES) {
    return json({ error: "request body too large" }, 413);
  }
  let raw;
  try {
    raw = await request.text();
  } catch {
    return json({ error: "unreadable request body" }, 400);
  }
  if (raw.length > MAX_BODY_BYTES) {
    return json({ error: "request body too large" }, 413);
  }
  if (new TextEncoder().encode(raw).byteLength > MAX_BODY_BYTES) {
    return json({ error: "request body too large" }, 413);
  }
  let payload;
  try {
    payload = JSON.parse(raw);
  } catch {
    return json({ error: "body must be JSON" }, 400);
  }
  if (payload === null || typeof payload !== "object" || Array.isArray(payload)) {
    return json({ error: "body must be a JSON object" }, 400);
  }
  if (typeof payload.message !== "string") {
    return json({ error: "message must be a string" }, 400);
  }
  const message = payload.message.trim();
  if (message.length === 0) {
    return json({ error: "message must not be empty" }, 400);
  }
  if (message.length > MAX_MESSAGE_CHARS) {
    return json({ error: `message must be at most ${MAX_MESSAGE_CHARS} characters` }, 400);
  }
  return json({
    capability: CAPABILITY_ID,
    ok: true,
    echo: message,
    context: { storeId: trustedStoreId(request) },
  });
}

async function capability(request, env) {
  const url = new URL(request.url);
  const route = url.pathname.slice("/capabilities/livechat".length) || "/";
  if (route === "/") {
    if (request.method !== "GET") return methodNotAllowed(["GET"]);
    return json({
      capability: CAPABILITY_ID,
      version: CAPABILITY_VERSION,
      endpoints: {
        session: "/capabilities/livechat/session",
        messages: "/capabilities/livechat/messages",
      },
    });
  }
  if (route === "/session") {
    if (request.method !== "GET") return methodNotAllowed(["GET"]);
    return session(request, env);
  }
  if (route === "/messages") {
    if (request.method !== "POST") return methodNotAllowed(["POST"]);
    return messages(request);
  }
  return json({ error: "not found" }, 404);
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/admin/conversations.list") {
      return Response.json({
        plugin: env.FASTCART_PLUGIN_ID,
        conversations: [
          { id: "c_1", customer: "Ada", last: "Where is my order?", unread: 2 },
          { id: "c_2", customer: "Grace", last: "Thanks!", unread: 0 },
        ],
      });
    }
    if (
      url.pathname === "/capabilities/livechat" ||
      url.pathname.startsWith("/capabilities/livechat/")
    ) {
      return capability(request, env);
    }
    if (url.pathname.startsWith("/api/")) {
      return Response.json({ ok: true, path: url.pathname });
    }
    return new Response("not found", { status: 404 });
  },
};
