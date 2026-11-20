export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/admin/conversations.list") {
      return Response.json({
        plugin: env.FASTCART_PLUGIN_ID,
        conversations: [
          { id: "c_1", customer: "Ada", last: "Where is my order?", unread: 2 },
          { id: "c_2", customer: "Grace", last: "Thanks!", unread: 0 },
        ],
      });
    }
    if (url.pathname.startsWith("/api/")) {
      return Response.json({ ok: true, path: url.pathname });
    }
    return new Response("not found", { status: 404 });
  },
};
