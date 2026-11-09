// Phase 0 service-binding caller: proves that a Worker can call another
// workerd service without exposing that service on a public socket.
export default {
  async fetch(request, env) {
    const response = await env.API.fetch(
      new Request("http://api/internal/ssr/v1/stores/store_123/products", {
        headers: { "x-fastcart-store-id": "store_123" },
      }),
    );
    return Response.json({
      from: "app",
      status: response.status,
      api: await response.json(),
    });
  },
};
