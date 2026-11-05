// Fastcart theme runtime sidecar.
//
// The Rust `NodeRuntime` talks to this process over a small JSON+base64 HTTP
// contract:
//
// - GET  /health                 readiness probe
// - POST /render                 render one storefront request
// - GET  /theme-assets/<bundle>/<client-relative-path>
//                                serve a file from the bundle's client output
//
// Environment:
// - PORT            listener port (default 8788)
// - HOST            listener host (default 127.0.0.1; set 0.0.0.0 in containers)
// - THEME_ROOT      directory containing theme bundles (default <repo>/themes;
//                   relative values resolve from the repository root)
// - SIDECAR_SECRET  optional bearer token required on /render

import { timingSafeEqual } from "node:crypto";
import { createReadStream, promises as fs } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const PACKAGE_ROOT = path.resolve(HERE, "..");
const REPO_ROOT = path.resolve(PACKAGE_ROOT, "../..");

const PORT = parsePort(process.env.PORT, 8788);
const HOST = process.env.HOST?.trim() || "127.0.0.1";
const THEME_ROOT = resolveFromRepo(process.env.THEME_ROOT, "themes");
const SECRET = process.env.SIDECAR_SECRET?.trim() || null;

const MAX_PAYLOAD_BYTES = 32 * 1024 * 1024;
const MAX_RESPONSE_BYTES = 10 * 1024 * 1024;
const MODULE_CACHE_LIMIT = 64;
const ASSET_CACHE_CONTROL = "public, max-age=3600";

/** Dynamic theme modules keyed by absolute path + mtime. */
const moduleCache = new Map();

const CONTENT_TYPES = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".mjs", "text/javascript; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".map", "application/json; charset=utf-8"],
  [".txt", "text/plain; charset=utf-8"],
  [".svg", "image/svg+xml"],
  [".png", "image/png"],
  [".jpg", "image/jpeg"],
  [".jpeg", "image/jpeg"],
  [".gif", "image/gif"],
  [".webp", "image/webp"],
  [".avif", "image/avif"],
  [".ico", "image/x-icon"],
  [".woff", "font/woff"],
  [".woff2", "font/woff2"],
  [".ttf", "font/ttf"],
  [".otf", "font/otf"],
  [".wasm", "application/wasm"],
]);

class HttpError extends Error {
  constructor(status, code, message) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

const server = createServer((request, response) => {
  handle(request, response).catch((error) => sendError(response, error));
});

server.listen(PORT, HOST, () => {
  console.log(`theme sidecar listening on http://${HOST}:${PORT}`);
  console.log(`theme root: ${THEME_ROOT}`);
  console.log(
    SECRET
      ? "render authentication: bearer secret enabled"
      : "render authentication: disabled (set SIDECAR_SECRET to enable)",
  );
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close(() => process.exit(0));
  });
}

async function handle(request, response) {
  const url = new URL(request.url ?? "/", `http://${request.headers.host ?? "localhost"}`);

  if (url.pathname === "/health") {
    if (request.method !== "GET" && request.method !== "HEAD") {
      throw new HttpError(405, "METHOD_NOT_ALLOWED", "use GET /health");
    }
    sendJson(response, 200, { status: "ok" });
    return;
  }

  if (url.pathname === "/render") {
    if (request.method !== "POST") {
      throw new HttpError(405, "METHOD_NOT_ALLOWED", "use POST /render");
    }
    if (!authorized(request)) {
      throw new HttpError(401, "UNAUTHORIZED", "missing or invalid bearer token");
    }
    const payload = await readJson(request);
    const result = await renderTheme(payload);
    sendJson(response, 200, result);
    return;
  }

  if (url.pathname.startsWith("/theme-assets/")) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      throw new HttpError(405, "METHOD_NOT_ALLOWED", "use GET /theme-assets/...");
    }
    await serveAsset(request, response, url);
    return;
  }

  throw new HttpError(404, "NOT_FOUND", "unknown route");
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

async function renderTheme(payload) {
  if (!isRecord(payload)) {
    throw new HttpError(400, "INVALID_PAYLOAD", "request body must be a JSON object");
  }
  const theme = payload.theme;
  if (!isRecord(theme)) {
    throw new HttpError(400, "INVALID_PAYLOAD", "payload.theme is required");
  }
  const bundlePath = relativePath(theme.bundle_path);
  if (bundlePath === null) {
    throw new HttpError(400, "INVALID_THEME", "theme.bundle_path must be a relative path");
  }
  const entry = relativePath(theme.entry);
  if (entry === null) {
    throw new HttpError(400, "INVALID_THEME", "theme.entry must be a relative path");
  }

  const themeRootReal = await realThemeRoot();
  const entryFile = await resolveThemeFile(themeRootReal, bundlePath, entry);
  const stat = await fs.stat(entryFile);
  if (!stat.isFile()) {
    throw new HttpError(400, "INVALID_THEME", "theme entry is not a file");
  }

  const module = await loadThemeModule(entryFile, stat);
  const render = renderFunction(module);
  if (!render) {
    throw new HttpError(500, "INVALID_THEME", "theme entry does not export a render function");
  }

  let result;
  try {
    result = await render(payload);
  } catch (error) {
    // Keep the stack in the sidecar log only; the host gets a stable error.
    console.error(`theme render failed for ${bundlePath}/${entry}:`, error);
    throw new HttpError(500, "RENDER_FAILED", "theme render failed");
  }
  return encodeResult(result);
}

async function loadThemeModule(entryFile, stat) {
  const key = `${entryFile}@${stat.mtimeMs}`;
  const cached = moduleCache.get(key);
  if (cached !== undefined) {
    return cached;
  }
  const url = `${pathToFileURL(entryFile).href}?mtime=${encodeURIComponent(String(stat.mtimeMs))}`;
  const module = await import(url);
  moduleCache.set(key, module);
  while (moduleCache.size > MODULE_CACHE_LIMIT) {
    const oldest = moduleCache.keys().next().value;
    if (oldest === undefined) {
      break;
    }
    moduleCache.delete(oldest);
  }
  return module;
}

function renderFunction(module) {
  if (typeof module?.render === "function") {
    return module.render;
  }
  if (typeof module?.default?.render === "function") {
    return module.default.render;
  }
  if (typeof module?.default === "function") {
    return module.default;
  }
  return null;
}

function encodeResult(result) {
  if (!isRecord(result)) {
    throw new HttpError(502, "INVALID_RESULT", "theme returned a non-object result");
  }
  const status = result.status;
  if (!Number.isInteger(status) || status < 100 || status > 599) {
    throw new HttpError(502, "INVALID_RESULT", "theme returned an invalid status");
  }
  const headers = result.headers ?? [];
  if (!Array.isArray(headers) || !headers.every(isHeaderTuple)) {
    throw new HttpError(502, "INVALID_RESULT", "theme returned invalid headers");
  }

  let body = null;
  if (result.body !== null && result.body !== undefined) {
    let bytes;
    if (typeof result.body === "string") {
      bytes = Buffer.from(result.body, "utf8");
    } else if (result.body instanceof Uint8Array) {
      bytes = Buffer.from(result.body);
    } else {
      throw new HttpError(502, "INVALID_RESULT", "theme returned an invalid body");
    }
    if (bytes.length > MAX_RESPONSE_BYTES) {
      throw new HttpError(502, "RESPONSE_TOO_LARGE", "theme response exceeded 10 MiB");
    }
    body = bytes.toString("base64");
  }

  return { status, headers, body };
}

function isHeaderTuple(value) {
  return (
    Array.isArray(value) &&
    value.length === 2 &&
    typeof value[0] === "string" &&
    typeof value[1] === "string" &&
    value[0].length > 0 &&
    !value[0].includes(":") &&
    !/[\r\n]/.test(value[0]) &&
    !/[\r\n]/.test(value[1])
  );
}

// ---------------------------------------------------------------------------
// Theme assets
// ---------------------------------------------------------------------------

async function serveAsset(request, response, url) {
  let rest;
  try {
    rest = decodeURIComponent(url.pathname.slice("/theme-assets/".length));
  } catch {
    throw new HttpError(400, "INVALID_PATH", "invalid asset path");
  }
  const segments = rest.split("/").filter((segment) => segment.length > 0);
  if (segments.length < 2) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }
  // Bundle paths are shallow. Rejecting long paths bounds the prefix probes
  // below, so one request cannot trigger thousands of filesystem syscalls.
  if (segments.length > 16) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }

  const themeRootReal = await realThemeRoot();
  const located = await locateBundle(themeRootReal, segments);
  if (!located) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }
  const { bundleDir, relative } = located;
  const clientDir = await readClientDir(bundleDir);
  const clientRoot = await resolveInside(bundleDir, clientDir);
  if (!clientRoot) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }
  const clientRootReal = await fs.realpath(clientRoot).catch(() => null);
  if (!clientRootReal) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }

  const target = await resolveInside(clientRootReal, relative);
  if (!target) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }
  const targetReal = await fs.realpath(target).catch(() => null);
  if (!targetReal || !isInside(clientRootReal, targetReal)) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }
  const stat = await fs.stat(targetReal).catch(() => null);
  if (!stat || !stat.isFile()) {
    throw new HttpError(404, "NOT_FOUND", "theme asset not found");
  }

  const contentType =
    CONTENT_TYPES.get(path.extname(targetReal).toLowerCase()) ?? "application/octet-stream";
  response.statusCode = 200;
  response.setHeader("content-type", contentType);
  response.setHeader("content-length", String(stat.size));
  response.setHeader("cache-control", ASSET_CACHE_CONTROL);
  response.setHeader("x-content-type-options", "nosniff");
  if (request.method === "HEAD") {
    response.end();
    return;
  }
  createReadStream(targetReal).pipe(response);
}

/**
 * Finds the bundle prefix of an asset URL.
 *
 * Bundle paths may contain several segments (for example `themes/default`),
 * while the remainder is relative to the bundle's client output. The longest
 * prefix that is a directory containing `theme.json` wins.
 */
async function locateBundle(themeRootReal, segments) {
  for (let index = segments.length - 1; index >= 1; index -= 1) {
    const candidate = segments.slice(0, index).join("/");
    if (relativePath(candidate) === null) {
      continue;
    }
    const bundleDir = await resolveInside(themeRootReal, candidate);
    if (!bundleDir) {
      continue;
    }
    const bundleReal = await fs.realpath(bundleDir).catch(() => null);
    if (!bundleReal || !isInside(themeRootReal, bundleReal)) {
      continue;
    }
    const manifest = await fs.stat(path.join(bundleReal, "theme.json")).catch(() => null);
    if (!manifest?.isFile()) {
      continue;
    }
    const relative = relativePath(segments.slice(index).join("/"));
    if (relative === null) {
      continue;
    }
    return { bundleDir: bundleReal, relative };
  }
  return null;
}

async function readClientDir(bundleDir) {
  const fallback = "dist/client";
  const manifestPath = await resolveInside(bundleDir, "assets.json");
  if (!manifestPath) {
    return fallback;
  }
  const manifestReal = await fs.realpath(manifestPath).catch(() => null);
  if (!manifestReal || !isInside(bundleDir, manifestReal)) {
    return fallback;
  }
  const raw = await fs.readFile(manifestReal, "utf8").catch(() => null);
  if (raw === null) {
    return fallback;
  }
  let manifest;
  try {
    manifest = JSON.parse(raw);
  } catch {
    return fallback;
  }
  const entry = manifest?.entries?.client;
  const value =
    typeof entry === "string"
      ? entry
      : isRecord(entry) && typeof entry.path === "string"
        ? entry.path
        : fallback;
  return relativePath(value) ?? fallback;
}

// ---------------------------------------------------------------------------
// Filesystem and HTTP helpers
// ---------------------------------------------------------------------------

async function realThemeRoot() {
  const real = await fs.realpath(THEME_ROOT).catch(() => null);
  if (!real) {
    throw new HttpError(500, "THEME_ROOT_UNAVAILABLE", "theme root is not available");
  }
  return real;
}

async function resolveThemeFile(themeRootReal, bundlePath, entry) {
  const bundleDir = await resolveInside(themeRootReal, bundlePath);
  if (!bundleDir) {
    throw new HttpError(403, "UNSAFE_PATH", "theme bundle is outside the theme root");
  }
  const bundleReal = await fs.realpath(bundleDir).catch(() => null);
  if (!bundleReal || !isInside(themeRootReal, bundleReal)) {
    throw new HttpError(404, "THEME_NOT_FOUND", "theme bundle was not found");
  }
  const entryFile = await resolveInside(bundleReal, entry);
  if (!entryFile) {
    throw new HttpError(403, "UNSAFE_PATH", "theme entry is outside the bundle");
  }
  const entryReal = await fs.realpath(entryFile).catch(() => null);
  if (!entryReal || !isInside(bundleReal, entryReal)) {
    throw new HttpError(404, "THEME_ENTRY_NOT_FOUND", "theme entry was not found");
  }
  return entryReal;
}

/** Resolves a relative path under `root`, rejecting traversal and escapes. */
async function resolveInside(root, relative) {
  if (relativePath(relative) === null) {
    return null;
  }
  const resolved = path.resolve(root, relative);
  return isInside(root, resolved) ? resolved : null;
}

function isInside(root, candidate) {
  const relative = path.relative(root, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

/** Returns a normalized relative path, or null when unsafe. */
function relativePath(value) {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (trimmed.length === 0 || trimmed.length > 1024) {
    return null;
  }
  if (
    trimmed.startsWith("/") ||
    trimmed.startsWith("\\") ||
    trimmed.includes("\\") ||
    trimmed.includes("\0") ||
    /^[A-Za-z]:/.test(trimmed)
  ) {
    return null;
  }
  const segments = trimmed.split("/");
  if (segments.some((segment) => segment === "" || segment === "." || segment === "..")) {
    return null;
  }
  return segments.join("/");
}

function authorized(request) {
  if (SECRET === null) {
    return true;
  }
  const header = request.headers.authorization;
  if (typeof header !== "string" || !header.startsWith("Bearer ")) {
    return false;
  }
  const provided = Buffer.from(header.slice("Bearer ".length).trim());
  const expected = Buffer.from(SECRET);
  return provided.length === expected.length && timingSafeEqual(provided, expected);
}

async function readJson(request) {
  let size = 0;
  const chunks = [];
  for await (const chunk of request) {
    size += chunk.length;
    if (size > MAX_PAYLOAD_BYTES) {
      throw new HttpError(413, "PAYLOAD_TOO_LARGE", "request body exceeded 32 MiB");
    }
    chunks.push(chunk);
  }
  if (size === 0) {
    throw new HttpError(400, "INVALID_JSON", "request body is empty");
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw new HttpError(400, "INVALID_JSON", "request body must be valid JSON");
  }
}

function sendJson(response, status, value) {
  const body = Buffer.from(JSON.stringify(value), "utf8");
  response.statusCode = status;
  response.setHeader("content-type", "application/json; charset=utf-8");
  response.setHeader("content-length", String(body.length));
  response.setHeader("x-content-type-options", "nosniff");
  response.end(body);
}

function sendError(response, error) {
  if (response.headersSent) {
    response.destroy();
    return;
  }
  if (error instanceof HttpError) {
    sendJson(response, error.status, {
      error: { code: error.code, message: error.message },
    });
    return;
  }
  console.error("theme sidecar request failed:", error);
  sendJson(response, 500, {
    error: { code: "INTERNAL_ERROR", message: "internal server error" },
  });
}

function resolveFromRepo(value, fallback) {
  const raw = value?.trim();
  if (!raw) {
    return path.resolve(REPO_ROOT, fallback);
  }
  return path.isAbsolute(raw) ? path.resolve(raw) : path.resolve(REPO_ROOT, raw);
}

function parsePort(value, fallback) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 && parsed < 65536 ? parsed : fallback;
}

function isRecord(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
