let calls = 0;

export default {
  async fetch(request) {
    calls += 1;
    const url = new URL(request.url);
    if (url.pathname === "/state") return Response.json({ calls });
    return new Response("hello");
  },
};
