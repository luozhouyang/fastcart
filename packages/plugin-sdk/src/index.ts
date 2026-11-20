/**
 * FastCart plugin SDK.
 *
 * Plugin workers receive the raw request and `env`. The host injects a
 * short-lived context token in `x-fastcart-context-token`; every call the
 * plugin makes must carry it through the bridge.
 */

export interface PluginContextInfo {
  plugin: string;
  store: {
    id: string;
    locale: string;
    currency: string;
    timezone: string;
  };
  scopes: string[];
  settings: Record<string, unknown>;
}

export interface KvStore {
  get(namespace: string, key: string): Promise<string | null>;
  set(namespace: string, key: string, value: string): Promise<void>;
  delete(namespace: string, key: string): Promise<void>;
}

export interface EgressOptions {
  method?: string;
  headers?: Array<[string, string]>;
  body?: string;
  bodyBase64?: string;
}

export interface EgressResult {
  status: number;
  content_type: string | null;
  /** Base64-encoded response body. */
  body: string;
}

export interface PluginContext {
  /** Context token for this request. Never log it. */
  readonly token: string;
  /** Calls the host API under `/internal/plugins/v1`. */
  api(path: string, init?: RequestInit): Promise<Response>;
  /** Loads plugin, store and settings for this request. */
  context(): Promise<PluginContextInfo>;
  kv: KvStore;
  /** Calls an allowlisted host through the host egress proxy. */
  egress(url: string, options?: EgressOptions): Promise<EgressResult>;
  log(...args: unknown[]): void;
}

export interface PluginHandlers {
  fetch(ctx: PluginContext, request: Request): Response | Promise<Response>;
  admin?(ctx: PluginContext, method: string, params: unknown): unknown;
  events?(ctx: PluginContext, event: unknown): unknown;
  jobs?(ctx: PluginContext, job: unknown): unknown;
}

const API_ORIGIN = "http://api";
const API_PREFIX = "/internal/plugins/v1";

export function createPluginContext(request: Request): PluginContext {
  const token = request.headers.get("x-fastcart-context-token") ?? "";
  if (!token) {
    throw new Error("plugin context token is missing");
  }
  const headers = { authorization: `Bearer ${token}` };

  const api = (path: string, init: RequestInit = {}) =>
    fetch(`${API_ORIGIN}${API_PREFIX}${path}`, {
      ...init,
      headers: {
        ...headers,
        "x-fastcart-call-id": crypto.randomUUID(),
        ...(init.headers ?? {}),
      },
    });

  return {
    token,
    api,
    async context() {
      const response = await api("/context");
      if (!response.ok) {
        throw new Error(`context request failed (${response.status})`);
      }
      return (await response.json()) as PluginContextInfo;
    },
    kv: {
      async get(namespace, key) {
        const response = await api(
          `/kv/${encodeURIComponent(namespace)}/${encodeURIComponent(key)}`,
        );
        if (response.status === 404) return null;
        if (!response.ok) {
          throw new Error(`kv get failed (${response.status})`);
        }
        return response.text();
      },
      async set(namespace, key, value) {
        const response = await api(
          `/kv/${encodeURIComponent(namespace)}/${encodeURIComponent(key)}`,
          { method: "PUT", body: value },
        );
        if (!response.ok) {
          throw new Error(`kv set failed (${response.status})`);
        }
      },
      async delete(namespace, key) {
        const response = await api(
          `/kv/${encodeURIComponent(namespace)}/${encodeURIComponent(key)}`,
          { method: "DELETE" },
        );
        if (!response.ok) {
          throw new Error(`kv delete failed (${response.status})`);
        }
      },
    },
    async egress(url, options = {}) {
      const response = await api("/egress", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          url,
          method: options.method ?? "GET",
          headers: options.headers ?? [],
          body: options.body,
          body_base64: options.bodyBase64,
        }),
      });
      if (!response.ok) {
        throw new Error(`egress failed (${response.status})`);
      }
      return (await response.json()) as EgressResult;
    },
    log(...args) {
      console.log("[fastcart-plugin]", ...args);
      const message = args
        .map((value) => (typeof value === "string" ? value : JSON.stringify(value)))
        .join(" ")
        .slice(0, 4096);
      void api("/log", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ level: "info", message }),
      }).catch(() => {
        // Logging is best effort and never breaks the request.
      });
    },
  };
}

/** Identity helper that preserves handler types for editors. */
export function definePlugin<T extends PluginHandlers>(handlers: T): T {
  return handlers;
}
