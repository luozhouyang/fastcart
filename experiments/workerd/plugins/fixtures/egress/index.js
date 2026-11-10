export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/fetch") {
      try {
        await fetch("https://example.com");
        return Response.json({ blocked: false });
      } catch (error) {
        return Response.json({ blocked: true, error: String(error) });
      }
    }
    if (url.pathname === "/ws") {
      try {
        const socket = new WebSocket("wss://example.com");
        socket.close();
        return Response.json({ blocked: false });
      } catch (error) {
        return Response.json({ blocked: true, error: String(error) });
      }
    }
    return new Response("ok");
  },
};
