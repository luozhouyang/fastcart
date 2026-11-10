export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/loop") {
      const started = Date.now();
      while (Date.now() - started < 300) {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      return Response.json({ finished: true, elapsed: Date.now() - started });
    }
    return new Response("ok");
  },
};
