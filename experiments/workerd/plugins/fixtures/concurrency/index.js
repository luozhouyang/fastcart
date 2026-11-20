let active = 0;
let maxActive = 0;

export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/reset") {
      maxActive = 0;
      return Response.json({ ok: true });
    }
    if (url.pathname === "/serial") {
      active += 1;
      maxActive = Math.max(maxActive, active);
      await new Promise((resolve) => setTimeout(resolve, 100));
      active -= 1;
      return Response.json({ maxActive });
    }
    return new Response("ok");
  },
};
