/** Minimal plugin entry. */
export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    return Response.json({
      ok: true,
      path: url.pathname,
      message: "Hello from your FastCart plugin",
    });
  },
};
