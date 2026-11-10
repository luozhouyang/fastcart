export default {
  async fetch(request, env) {
    const response = await env.FASTCART_API.fetch("http://bridge/internal/plugins/v1/context");
    const upstream = await response.json();
    return Response.json({
      plugin: env.FASTCART_PLUGIN_ID,
      upstream,
      envKeys: Object.keys(env).sort(),
    });
  },
};
