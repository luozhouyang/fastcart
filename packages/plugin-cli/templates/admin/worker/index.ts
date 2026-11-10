/** Handles the admin RPC allowlisted in plugin.json. */
export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/admin/panel.info") {
      return Response.json({ ok: true, method: "panel.info" });
    }
    return new Response("not found", { status: 404 });
  },
};
