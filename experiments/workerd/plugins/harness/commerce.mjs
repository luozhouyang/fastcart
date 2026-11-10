import { createHash } from "node:crypto";
import { createServer } from "node:http";

export function fixture(id, modules, options = {}) {
  const mainModule = options.mainModule ?? "index.js";
  const digest =
    options.digest ??
    createHash("sha256").update(JSON.stringify(modules)).digest("hex").slice(0, 16);
  const bundle = {
    compatibilityDate: options.compatibilityDate ?? "2026-11-01",
    mainModule,
    modules,
  };
  if (options.flags) bundle.compatibilityFlags = options.flags;
  if (options.limits) bundle.limits = options.limits;
  if (options.entryLimits) bundle.entryLimits = options.entryLimits;
  if (options.env) bundle.env = options.env;
  return { id, digest, bundle };
}

export async function startCommerce(fixtures) {
  const byId = new Map();
  for (const item of fixtures) {
    if (!byId.has(item.id)) byId.set(item.id, new Map());
    byId.get(item.id).set(item.digest, item.bundle);
  }

  const requests = [];
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, "http://commerce");
    const body = await readBody(request);
    requests.push({
      method: request.method,
      path: url.pathname,
      query: Object.fromEntries(url.searchParams),
      headers: request.headers,
      body: body.length <= 64 * 1024 ? body.toString("utf8") : `<${body.length} bytes>`,
    });

    const bundle = url.pathname.match(/^\/internal\/plugins\/v1\/runtime\/bundles\/([a-z0-9-]+)$/);
    if (bundle) {
      const digest = url.searchParams.get("digest");
      const found = byId.get(bundle[1])?.get(digest);
      if (!found) {
        json(response, 404, { error: "bundle not found" });
        return;
      }
      json(response, 200, found);
      return;
    }

    if (url.pathname === "/internal/plugins/v1/context") {
      json(response, 200, { store: "store_p0", locale: "en-US", currency: "USD" });
      return;
    }

    if (url.pathname === "/internal/plugins/v1/egress") {
      json(response, 200, { ok: true, host: "example.com" });
      return;
    }

    if (url.pathname === "/_debug/requests") {
      json(response, 200, requests);
      return;
    }

    if (url.pathname === "/_debug/reset") {
      requests.length = 0;
      json(response, 200, { ok: true });
      return;
    }

    json(response, 404, { error: "not found" });
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();

  return {
    port,
    requests,
    async stop() {
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

function readBody(request) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => resolve(Buffer.concat(chunks)));
    request.on("error", reject);
  });
}

function json(response, status, value) {
  const payload = JSON.stringify(value);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  });
  response.end(payload);
}
