import { spawn } from "node:child_process";
import { readFile, readdir, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { join, relative } from "node:path";

const artifactRoot = process.argv[2] ?? new URL("../dist/artifacts", import.meta.url).pathname;
const artifact = await resolveArtifact(artifactRoot);
const binary =
  process.env.FASTCART_WORKERD_BIN ??
  new URL("../../../experiments/workerd/bin/workerd", import.meta.url).pathname;

try {
  const manifest = JSON.parse(await readFile(join(artifact, "fastcart-ssr.json"), "utf8"));
  const port = await freePort();
  const modules = await listModules(artifact, artifact);
  const entry = manifest.entry.path;
  modules.sort((left, right) =>
    left === entry ? -1 : right === entry ? 1 : left.localeCompare(right),
  );
  const moduleText = modules
    .map(
      (name) => `    (name = ${JSON.stringify(name)}, esModule = embed ${JSON.stringify(name)}),`,
    )
    .join("\n");
  await writeFile(
    join(artifact, "workerd.capnp"),
    `using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "cache", worker = .cacheWorker),
    (name = "assets", disk = (path = "client", writable = false)),
    (name = "main", worker = .mainWorker),
  ],
  sockets = [(name = "http", address = "127.0.0.1:${port}", http = (), service = "main")],
);

const mainWorker :Workerd.Worker = (
  cacheApiOutbound = "cache",
  bindings = [(name = "ASSETS", service = "assets")],
  modules = [
${moduleText}
  ],
  compatibilityDate = ${JSON.stringify(manifest.runtime.compatibility_date)},
  compatibilityFlags = [${manifest.runtime.compatibility_flags.map(JSON.stringify).join(", ")}],
);

const cacheWorker :Workerd.Worker = (
  modules = [
    (name = "cache.js", esModule = "export default { fetch() { return new Response(null, { status: 404 }); } };"),
  ],
  compatibilityDate = "2026-11-01",
);
`,
  );

  const child = spawn(binary, ["serve", join(artifact, "workerd.capnp")], {
    cwd: artifact,
    stdio: ["ignore", "inherit", "inherit"],
  });
  try {
    await waitFor(`http://127.0.0.1:${port}/__fastcart/health`);
    const base = `http://127.0.0.1:${port}`;

    const home = await fetch(`${base}/`);
    assert(home.status === 200, `home status ${home.status}`);
    assert((await home.text()).includes("FastCart SvelteKit SSR"), "home body");

    const echo = await fetch(`${base}/echo`, {
      method: "POST",
      headers: {
        "x-fastcart-store-id": "store_trusted",
        "x-fastcart-deployment-id": "deploy_1",
        "x-fastcart-request-id": "req_1",
        "x-forwarded-proto": "https",
      },
      body: "hello",
    });
    const echoed = await echo.json();
    assert(echoed.store === "store_trusted", "trusted store header");
    assert(echoed.body === "hello", "request body");

    const streamStart = Date.now();
    const stream = await fetch(`${base}/stream`);
    const reader = stream.body.getReader();
    let chunks = 0;
    for (;;) {
      const { done } = await reader.read();
      if (done) break;
      chunks += 1;
    }
    assert(chunks >= 3, `stream chunks ${chunks}`);
    assert(Date.now() - streamStart >= 250, "streaming was buffered");

    const cookies = await fetch(`${base}/cookie`);
    assert(cookies.headers.getSetCookie().length === 2, "duplicate set-cookie");

    const redirect = await fetch(`${base}/redirect`, { redirect: "manual" });
    assert(redirect.status === 302, `redirect status ${redirect.status}`);
    assert(redirect.headers.get("location") === "/", "redirect location");

    const boom = await fetch(`${base}/boom`);
    assert([200, 500].includes(boom.status), `error status ${boom.status}`);

    console.log("sveltekit artifact smoke passed");
  } finally {
    child.kill("SIGTERM");
    await new Promise((resolve) => child.once("exit", resolve));
  }
} catch (error) {
  if (error?.code === "ENOENT") {
    console.error("workerd binary not found; skipping tanstack artifact smoke");
    process.exit(0);
  }
  throw error;
}

async function resolveArtifact(path) {
  if (await exists(join(path, "fastcart-ssr.json"))) return path;
  const entries = await readdir(path, { withFileTypes: true });
  const candidates = entries.filter((entry) => entry.isDirectory());
  for (const entry of candidates) {
    const candidate = join(path, entry.name);
    if (await exists(join(candidate, "fastcart-ssr.json"))) return candidate;
  }
  throw new Error(`no artifact manifest under ${path}`);
}

async function exists(path) {
  try {
    await readFile(path);
    return true;
  } catch {
    return false;
  }
}

async function listModules(directory, root) {
  const output = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "client") continue;
      output.push(...(await listModules(path, root)));
    } else if (entry.isFile() && /\.(?:js|mjs)$/.test(entry.name)) {
      output.push(relative(root, path).split("\\").join("/"));
    }
  }
  return output;
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
  });
}

async function waitFor(url) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {
      // The runtime may still be starting; retry until the deadline.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`runtime did not become ready at ${url}`);
}

function assert(condition, message) {
  if (!condition) throw new Error(`assertion failed: ${message}`);
}
