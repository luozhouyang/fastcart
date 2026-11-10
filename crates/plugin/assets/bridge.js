// FastCart shared bridge: the only egress available to a plugin worker.
// It forwards to the host API prefix and strips browser credentials.
const PREFIX = "/internal/plugins/v1/";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!url.pathname.startsWith(PREFIX)) {
      return new Response("forbidden", { status: 403 });
    }
    const headers = new Headers(request.headers);
    headers.delete("cookie");
    headers.delete("x-fastcart-store-id");
    headers.delete("x-fastcart-plugin-id");
    headers.set("x-fastcart-bridge", "1");

    const init = { method: request.method, headers };
    if (request.method !== "GET" && request.method !== "HEAD") {
      init.body = request.body;
    }
    return env.UPSTREAM.fetch(
      new URL(url.pathname + url.search, "http://commerce"),
      init,
    );
  },
};
