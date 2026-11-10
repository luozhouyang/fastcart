export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/loop") {
      const ms = Number(url.searchParams.get("ms") ?? 300);
      const started = Date.now();
      while (Date.now() - started < ms) {
        // deliberate synchronous spin
      }
      return Response.json({ finished: true, elapsed: Date.now() - started });
    }
    return new Response("ok");
  },
};
