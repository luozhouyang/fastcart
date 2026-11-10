const modules = [
  "node:fs",
  "node:net",
  "node:tls",
  "node:child_process",
  "node:worker_threads",
  "node:dns",
];

export default {
  async fetch() {
    const result = {};
    for (const name of modules) {
      try {
        const module = await import(name);
        result[name] = { imported: true, keys: Object.keys(module).slice(0, 5) };
        if (name === "node:fs") {
          result[name].read = attempt(() => module.readFileSync("/etc/hostname", "utf8").length);
          result[name].list = attempt(() => module.readdirSync("/").slice(0, 5));
          result[name].write = attempt(() => module.writeFileSync("/tmp/fc-p0-fs-test", "x"));
        }
        if (name === "node:child_process") {
          result[name].exec = attempt(() => module.execSync("echo p0"));
          result[name].spawn = attempt(() => module.spawnSync("id"));
        }
        if (name === "node:net") {
          result[name].connect = await attemptAsync(
            () =>
              new Promise((resolve, reject) => {
                const socket = module.connect({ host: "127.0.0.1", port: 9 });
                socket.on("connect", () => {
                  socket.destroy();
                  resolve("connected");
                });
                socket.on("error", reject);
                setTimeout(() => reject(new Error("timeout")), 1000);
              }),
          );
        }
      } catch (error) {
        result[name] = { imported: false, error: String(error).slice(0, 120) };
      }
    }
    return Response.json(result);
  },
};

async function attemptAsync(fn) {
  try {
    const value = await fn();
    return { ok: true, value: String(value).slice(0, 40) };
  } catch (error) {
    return { ok: false, error: String(error).slice(0, 120) };
  }
}

function attempt(fn) {
  try {
    const value = fn();
    return {
      ok: true,
      value: typeof value === "string" ? value.slice(0, 40) : String(value).slice(0, 40),
    };
  } catch (error) {
    return { ok: false, error: String(error).slice(0, 120) };
  }
}
