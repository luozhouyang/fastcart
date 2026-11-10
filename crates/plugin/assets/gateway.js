// FastCart plugin gateway: resolves {plugin}@{digest}, loads it lazily through
// the Worker Loader, and forwards the request. The plugin never receives the
// admin session cookie or the runtime token.
const BUNDLE_ROOT = "/internal/plugins/v1/stores";

function jsonError(message, status) {
  return Response.json({ error: message }, { status });
}

async function loadBundle(env, store, id, digest) {
  const url = `http://commerce${BUNDLE_ROOT}/${encodeURIComponent(store)}/bundles/${encodeURIComponent(id)}?digest=${digest}`;
  const response = await env.COMMERCE.fetch(url, {
    headers: { authorization: `Bearer ${env.RUNTIME_TOKEN}` },
  });
  if (!response.ok) {
    throw new Error(`bundle ${id}@${digest} unavailable (${response.status})`);
  }
  return response.json();
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/__fastcart/health") {
      return Response.json({ status: "ok" });
    }
    if (request.headers.get("x-fastcart-gateway-token") !== env.RUNTIME_TOKEN) {
      return jsonError("unauthorized gateway call", 401);
    }
    const match = url.pathname.match(
      /^\/__plugin\/([a-z0-9][a-z0-9._-]{0,118}[a-z0-9])@([0-9a-f]{64})(\/.*)?$/,
    );
    if (!match) return new Response("not found", { status: 404 });
    const [, pluginId, digest, rest] = match;

    const storeId = request.headers.get("x-fastcart-store-id");
    if (!storeId) return jsonError("missing store context", 400);

    let bundle;
    try {
      bundle = await loadBundle(env, storeId, pluginId, digest);
    } catch (error) {
      return jsonError(String(error), 502);
    }

    let worker;
    try {
      worker = env.LOADER.get(`${pluginId}@${digest}`, () => {
        const code = {
          compatibilityDate: bundle.compatibilityDate,
          mainModule: bundle.mainModule,
          modules: bundle.modules,
          env: {
            FASTCART_API: env.BRIDGE,
            FASTCART_PLUGIN_ID: pluginId,
          },
          globalOutbound: null,
        };
        if (bundle.compatibilityFlags) {
          code.compatibilityFlags = bundle.compatibilityFlags;
        }
        return code;
      });
    } catch (error) {
      return jsonError(String(error), 502);
    }

    const target = new URL(request.url);
    target.pathname = rest || "/";
    const headers = new Headers(request.headers);
    // Defense in depth: the Rust host strips these before forwarding, but a
    // gateway is also reachable from tests and future callers.
    headers.delete("cookie");
    headers.delete("authorization");
    headers.delete("x-csrf-token");
    headers.set("x-fastcart-plugin-id", pluginId);

    const init = { method: request.method, headers };
    if (request.method !== "GET" && request.method !== "HEAD") {
      init.body = request.body;
    }
    try {
      return await worker.getEntrypoint().fetch(new Request(target, init));
    } catch (error) {
      return jsonError(String(error), 502);
    }
  },
};
