/** Local preview and rebuild loop for `fastcart-plugin dev`. */

import { spawn, type ChildProcess } from "node:child_process";
import { cp, mkdir, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { createConnection } from "node:net";
import { join, resolve } from "node:path";

import { build } from "esbuild";

import { readManifest, sourceEntry, type PluginManifest } from "./manifest.ts";

export interface DevOptions {
  root: string;
  out?: string;
  intervalMs?: number;
  /** Start workerd and serve the preview. Defaults to true. */
  serve?: boolean;
  port?: number;
  /** Path to the workerd binary. Defaults to `FASTCART_WORKERD` or `workerd`. */
  workerd?: string;
}

export interface DevSession {
  url?: string;
  out: string;
  stop: () => Promise<void>;
}

interface Site {
  out: string;
  manifest: PluginManifest;
  config: string;
}

/** Builds the bundle and preview files without starting workerd. */
export async function prepareDevSite(root: string, out: string): Promise<Site> {
  const manifest = await readManifest(root);
  const directory = resolve(out);
  await rm(directory, { recursive: true, force: true });
  const assets = join(directory, "assets");
  await mkdir(assets, { recursive: true });
  await build({
    absWorkingDir: resolve(root),
    entryPoints: [sourceEntry(root, manifest.entry.path)],
    outfile: join(directory, "plugin.js"),
    bundle: true,
    format: "esm",
    platform: "neutral",
    target: "es2022",
    external: ["node:*"],
    logLevel: "silent",
    sourcemap: false,
    minify: false,
    legalComments: "none",
  });
  for (const name of ["ui", "storefront"]) {
    const source = join(root, name);
    if (await exists(source)) await cp(source, join(assets, name), { recursive: true });
  }
  await writeFile(join(assets, "index.html"), indexHtml(manifest));
  await writeFile(join(assets, "admin.html"), adminHtml(manifest));
  await writeFile(join(assets, "storefront.html"), storefrontHtml(manifest));
  await writeFile(join(directory, "harness.js"), harness(manifest));
  const config = join(directory, "dev.capnp");
  await writeFile(config, capnp(manifest, directory, 8787));
  return { out: directory, manifest, config };
}

/** Starts the rebuild loop and, when enabled, a workerd preview. */
export async function devPlugin(options: DevOptions): Promise<DevSession> {
  const root = resolve(options.root);
  const out = resolve(options.out ?? join(root, ".fastcart"));
  const interval = options.intervalMs ?? 500;
  const serve = options.serve ?? true;
  const port = options.port ?? 8787;
  const workerd = options.workerd ?? process.env.FASTCART_WORKERD ?? "workerd";

  const site = await prepareDevSite(root, out);
  if (serve) {
    await writeFile(site.config, capnp(site.manifest, site.out, port));
  }
  let child: ChildProcess | undefined;
  let stopped = false;
  let fingerprint = await treeFingerprint(root, out);

  const spawnWorkerd = async (prepared: Site): Promise<void> => {
    child = spawn(workerd, ["serve", "--experimental", prepared.config], {
      stdio: ["ignore", "inherit", "inherit"],
    });
    child.on("exit", (code) => {
      if (!stopped) console.error(`[fastcart-plugin] workerd exited with ${code ?? "signal"}`);
    });
    await waitForPort(port, 15_000);
  };

  if (serve) await spawnWorkerd(site);
  else await writeSiteArtifact(root, out);

  const timer = setInterval(() => {
    void (async () => {
      const next = await treeFingerprint(root, out);
      if (next === fingerprint) return;
      fingerprint = next;
      try {
        if (!serve) {
          await prepareDevSite(root, out);
          await writeSiteArtifact(root, out);
          console.log("[fastcart-plugin] rebuilt");
          return;
        }
        if (child && child.exitCode === null) {
          child.kill("SIGTERM");
          await onceExit(child);
        }
        const prepared = await prepareDevSite(root, out);
        await writeFile(prepared.config, capnp(prepared.manifest, prepared.out, port));
        await spawnWorkerd(prepared);
        console.log(`[fastcart-plugin] reloaded http://127.0.0.1:${port}/`);
      } catch (error) {
        console.error(
          `[fastcart-plugin] ${error instanceof Error ? error.message : String(error)}`,
        );
      }
    })();
  }, interval);

  return {
    ...(serve ? { url: `http://127.0.0.1:${port}/` } : {}),
    out,
    stop: async () => {
      stopped = true;
      clearInterval(timer);
      if (child && child.exitCode === null) {
        child.kill("SIGTERM");
        await onceExit(child);
      }
    },
  };
}

async function writeSiteArtifact(root: string, out: string): Promise<void> {
  const { packPlugin } = await import("./pack.ts");
  await packPlugin({ root, out: join(out, "dist") });
}

function onceExit(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolvePromise) => child.once("exit", () => resolvePromise()));
}

async function waitForPort(port: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await canConnect(port)) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
  }
  throw new Error(`workerd did not listen on 127.0.0.1:${port}`);
}

function canConnect(port: number): Promise<boolean> {
  return new Promise((resolvePromise) => {
    const socket = createConnection({ host: "127.0.0.1", port });
    socket.once("connect", () => {
      socket.destroy();
      resolvePromise(true);
    });
    socket.once("error", () => resolvePromise(false));
  });
}

async function exists(path: string): Promise<boolean> {
  return stat(path)
    .then(() => true)
    .catch(() => false);
}

function indexHtml(manifest: PluginManifest): string {
  const admin = (manifest.extensions?.admin ?? [])
    .filter((extension) => extension.entry)
    .map(
      (extension) =>
        `<li><a href="/__preview/admin.html?entry=${encodeURIComponent(extension.entry ?? "")}">${escapeHtml(
          extension.title ?? extension.label ?? extension.entry ?? "admin",
        )}</a></li>`,
    )
    .join("");
  const storefront = (manifest.extensions?.storefront ?? [])
    .map(
      (extension) =>
        `<li><a href="/__preview/storefront.html?entry=${encodeURIComponent(extension.entry)}${extension.css ? `&css=${encodeURIComponent(extension.css)}` : ""}">${escapeHtml(extension.entry)}</a></li>`,
    )
    .join("");
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(manifest.name)} dev</title>
<style>body{font:14px/1.5 system-ui,sans-serif;max-width:760px;margin:40px auto;padding:0 20px;color:#111}
code{background:#f1f5f9;padding:2px 5px;border-radius:4px}a{color:#2563eb}li{margin:4px 0}</style></head>
<body><h1>${escapeHtml(manifest.name)}</h1><p><code>${escapeHtml(manifest.id)}@${escapeHtml(manifest.version)}</code></p>
<h2>Worker</h2><p><a href="/apps/${encodeURIComponent(manifest.id)}/">/apps/${escapeHtml(manifest.id)}/</a></p>
<h2>Admin</h2><ul>${admin || "<li>No admin extensions.</li>"}</ul>
<h2>Storefront</h2><ul>${storefront || "<li>No storefront extensions.</li>"}</ul>
</body></html>`;
}

function adminHtml(manifest: PluginManifest): string {
  const extensions = JSON.stringify(manifest.extensions?.admin ?? []).replaceAll("<", "\\u003c");
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Admin preview</title>
<style>html,body{height:100%%;margin:0}iframe{width:100%%;height:100%%;border:0}</style></head>
<body><iframe id="frame" sandbox="allow-scripts allow-forms"></iframe>
<script>
const id = ${JSON.stringify(manifest.id)};
const channel = "fc_dev";
const frame = document.getElementById("frame");
const entry = new URLSearchParams(location.search).get("entry");
const extensions = ${extensions};
const chosen = extensions.find((item) => item.entry === entry);
if (!chosen) { document.body.textContent = "unknown admin entry"; }
else frame.src = "/__preview/" + entry;
const send = (type, payload) => frame.contentWindow?.postMessage({ v: 1, type, channel, payload }, "*");
const init = () => send("init", {
  target: "dev",
  pluginId: id,
  store: { id: "dev", name: "Dev store", locale: "en" },
  theme: { mode: "light" },
  context: {},
  user: { id: "dev", name: "Dev", permissions: [] },
  capabilities: chosen?.settings ?? {},
});
frame.addEventListener("load", init);
window.addEventListener("message", async (event) => {
  if (event.source !== frame.contentWindow) return;
  const message = event.data;
  if (!message || message.v !== 1 || message.channel !== channel) return;
  if (message.type === "ready") { init(); return; }
  if (message.type === "resize") return;
  if (message.type !== "rpc" || !message.method) return;
  try {
    const response = await fetch("/apps/" + encodeURIComponent(id) + "/admin/" + message.method, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(message.params ?? {}),
    });
    const body = await response.json().catch(() => null);
    if (!response.ok) throw new Error(body?.error ?? "RPC failed (" + response.status + ")");
    send("rpc:result", { id: message.id, ok: true, result: body });
  } catch (error) {
    send("rpc:result", { id: message.id, ok: false, error: error instanceof Error ? error.message : "RPC failed" });
  }
});
</script></body></html>`;
}

function storefrontHtml(manifest: PluginManifest): string {
  const extensions = JSON.stringify(manifest.extensions?.storefront ?? []).replaceAll(
    "<",
    "\\u003c",
  );
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Storefront preview</title><link id="css" rel="stylesheet"></head>
<body><script>
const id = ${JSON.stringify(manifest.id)};
const extensions = ${extensions};
const params = new URLSearchParams(location.search);
const entry = params.get("entry");
const css = params.get("css");
if (css) document.getElementById("css").href = "/__preview/" + css;
window.FastCartExtensions = {
  config: () => ({ id, settings: {}, store: { id: "dev" } }),
  api: (path, init) => fetch(path, init),
  register: (tag, define) => { if (!customElements.get(tag)) customElements.define(tag, define); },
};
if (!extensions.some((item) => item.entry === entry)) {
  document.body.textContent = "unknown storefront entry";
} else {
  const script = document.createElement("script");
  script.type = "module";
  script.src = "/__preview/" + entry;
  document.body.appendChild(script);
}
</script></body></html>`;
}

function harness(manifest: PluginManifest): string {
  return `import plugin from "./plugin.js";
const id = ${JSON.stringify(manifest.id)};
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/" || url.pathname === "/__preview") {
      return env.ASSETS.fetch(new Request("http://assets/index.html"));
    }
    if (url.pathname.startsWith("/__preview/")) {
      const asset = new URL(url.pathname.slice("/__preview/".length), "http://assets");
      return env.ASSETS.fetch(new Request(asset, request));
    }
    if (url.pathname.startsWith("/apps/" + id)) {
      const rest = url.pathname.slice(("/apps/" + id).length) || "/";
      const target = new URL(rest + url.search, "http://plugin");
      return plugin.fetch(new Request(target, request), env, {});
    }
    return new Response("not found", { status: 404 });
  },
};
`;
}

function capnp(manifest: PluginManifest, out: string, port: number): string {
  return `using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "main", worker = .main),
    (name = "assets", disk = (path = ${capnpString(join(out, "assets"))}, writable = false)),
  ],
  sockets = [
    (name = "http", address = "127.0.0.1:${port}", http = (), service = "main"),
  ],
);

const main :Workerd.Worker = (
  modules = [
    (name = "harness.js", esModule = embed "harness.js"),
    (name = "plugin.js", esModule = embed "plugin.js"),
  ],
  compatibilityDate = ${capnpString(manifest.runtime.compatibility_date)},
  bindings = [(name = "ASSETS", service = "assets")],
);
`;
}

function capnpString(value: string): string {
  return JSON.stringify(value);
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (character) => {
    switch (character) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&#39;";
    }
  });
}

async function treeFingerprint(root: string, out: string, prefix = ""): Promise<string> {
  const entries = await readdir(join(root, prefix), { withFileTypes: true });
  const rows: string[] = [];
  for (const entry of entries) {
    if (entry.name === "node_modules" || entry.name === "dist" || entry.name === ".git") continue;
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (resolve(root, path) === out) continue;
    if (entry.isDirectory()) {
      rows.push(await treeFingerprint(root, out, path));
    } else if (entry.isFile()) {
      const info = await stat(join(root, path));
      rows.push(`${path}:${info.size}:${info.mtimeMs}`);
    }
  }
  return rows.join("|");
}

/** Reads a generated preview file; useful for tests and tooling. */
export async function readPreview(path: string): Promise<string> {
  return readFile(path, "utf8");
}
