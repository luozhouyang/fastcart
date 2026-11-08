// Startup bind guard for the theme sidecar.
//
// `/render` executes trusted theme code for every caller. Binding that surface
// to a non-loopback interface is only safe when callers must present the
// `SIDECAR_SECRET` bearer token, so the entrypoint refuses to start otherwise.

const LOOPBACK_HOSTS = new Set(["127.0.0.1", "::1", "localhost"]);

/**
 * Returns true when `host` only accepts local loopback connections.
 *
 * The listen host is used verbatim by `server.listen`, so aliases such as
 * `LOCALHOST` are normalized but address ranges (for example `127.0.0.2`) are
 * treated as non-loopback on purpose: such a bind still needs a secret.
 */
export function isLoopbackHost(host) {
  if (typeof host !== "string") {
    return false;
  }
  let normalized = host.trim().toLowerCase();
  if (normalized.startsWith("[") && normalized.endsWith("]")) {
    normalized = normalized.slice(1, -1);
  }
  return LOOPBACK_HOSTS.has(normalized);
}

/**
 * Throws when `host` is reachable beyond loopback and no secret is configured.
 *
 * An empty or whitespace-only secret is treated as unset.
 */
export function assertSafeBind(host, secret) {
  const trimmedSecret = typeof secret === "string" ? secret.trim() : "";
  if (isLoopbackHost(host) || trimmedSecret !== "") {
    return;
  }
  throw new Error(
    `refusing to start: HOST=${JSON.stringify(host)} is not loopback but SIDECAR_SECRET is empty. ` +
      "Set SIDECAR_SECRET to a strong random value, or bind HOST to 127.0.0.1, ::1 or localhost.",
  );
}
