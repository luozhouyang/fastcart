// P0 loader gateway: mirrors the production shape (route -> LOADER.get -> fetch).
const BUNDLES = "/internal/plugins/v1/runtime/bundles";
const cache = new Map();

async function getBundle(env, id, digest) {
  const key = `${id}@${digest}`;
  if (cache.has(key)) return cache.get(key);
  const response = await env.COMMERCE.fetch(`http://commerce${BUNDLES}/${id}?digest=${digest}`, {
    headers: { authorization: "Bearer p0-runtime-token" },
  });
  if (!response.ok) throw new Error(`bundle ${key} unavailable (${response.status})`);
  const bundle = await response.json();
  cache.set(key, bundle);
  return bundle;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/__fastcart/health") {
      return Response.json({ status: "ok" });
    }
    const match = url.pathname.match(/^\/p\/([a-z0-9-]+)@([0-9a-f]+)(\/.*)?$/);
    if (!match) return new Response("not found", { status: 404 });
    const [, id, digest, rest] = match;

    let bundle;
    try {
      bundle = await getBundle(env, id, digest);
    } catch (error) {
      return Response.json({ error: String(error) }, { status: 502 });
    }

    let worker;
    try {
      worker = env.LOADER.get(`${id}@${digest}`, () => {
        const code = {
          compatibilityDate: bundle.compatibilityDate,
          mainModule: bundle.mainModule,
          modules: bundle.modules,
          env: { FASTCART_API: env.BRIDGE, FASTCART_PLUGIN_ID: id, ...(bundle.env ?? {}) },
          globalOutbound: null,
        };
        if (bundle.compatibilityFlags) code.compatibilityFlags = bundle.compatibilityFlags;
        if (bundle.limits) code.limits = bundle.limits;
        return code;
      });
    } catch (error) {
      return Response.json({ error: String(error) }, { status: 502 });
    }

    const target = new URL(request.url);
    target.pathname = rest || "/";
    const headers = new Headers(request.headers);
    headers.set("x-fastcart-context-token", `p0-token-${id}`);
    headers.set("x-fastcart-store-id", "store_p0");

    try {
      const entrypoint = bundle.entryLimits
        ? worker.getEntrypoint(undefined, { limits: bundle.entryLimits })
        : worker.getEntrypoint();
      const forwarded = new Request(target, {
        method: request.method,
        headers,
        body: request.method === "GET" || request.method === "HEAD" ? undefined : request.body,
      });
      return await entrypoint.fetch(forwarded);
    } catch (error) {
      return Response.json({ error: String(error) }, { status: 502 });
    }
  },
};
