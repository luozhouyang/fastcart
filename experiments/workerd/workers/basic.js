// Phase 0 basic Worker: proves that a standard Fetch handler runs under workerd.
export default {
  async fetch(request) {
    const url = new URL(request.url);
    return Response.json({
      message: "FastCart SSR Runtime",
      method: request.method,
      path: url.pathname,
      search: url.search,
      hasCf: typeof request.cf === "object",
    });
  },
};
