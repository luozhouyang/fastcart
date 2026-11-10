import net from "node:net";

export default {
  async fetch(request, env) {
    const port = Number(env.PROBE_PORT);
    return await new Promise((resolve) => {
      const socket = net.connect({ host: "127.0.0.1", port });
      const timer = setTimeout(() => {
        socket.destroy();
        resolve(Response.json({ connected: false, error: "timeout" }));
      }, 1500);
      socket.on("connect", () => {
        clearTimeout(timer);
        socket.destroy();
        resolve(Response.json({ connected: true }));
      });
      socket.on("error", (error) => {
        clearTimeout(timer);
        resolve(Response.json({ connected: false, error: String(error).slice(0, 120) }));
      });
    });
  },
};
