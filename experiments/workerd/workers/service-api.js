// Phase 0 service-binding upstream: represents the internal Commerce API.
export default {
  async fetch(request) {
    const url = new URL(request.url);
    return Response.json({
      from: "api",
      path: url.pathname,
      storeId: request.headers.get("x-fastcart-store-id"),
    });
  },
};
