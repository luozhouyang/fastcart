export default {
  async fetch(request) {
    if (request.headers.get("upgrade") !== "websocket") {
      return new Response("expected upgrade", { status: 400 });
    }
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    server.accept();
    server.send("welcome");
    server.addEventListener("message", (event) => server.send(`echo:${event.data}`));
    return new Response(null, { status: 101, webSocket: client });
  },
};
