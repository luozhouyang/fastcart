export default {
  async fetch(request, env) {
    return Response.json({
      token: request.headers.get("x-fastcart-context-token"),
      store: request.headers.get("x-fastcart-store-id"),
      plugin: env.FASTCART_PLUGIN_ID,
      url: new URL(request.url).pathname,
    });
  },
};
