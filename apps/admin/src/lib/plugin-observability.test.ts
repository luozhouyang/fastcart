import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { API_BASE_URL } from "@/lib/api";
import {
  clearLogs,
  getPlugin,
  isActiveStatus,
  listDeliveries,
  listJobs,
  listLogs,
  parseLogFields,
  replayDelivery,
  rollbackPlugin,
  runJob,
  runtimeStatus,
  statusTone,
  upgradePlugin,
  type DeliveryView,
  type JobView,
  type LogView,
  type PluginDetail,
  type PluginRuntimeStatus,
} from "@/lib/plugin-observability";

function jsonResponse(body: unknown, status = 200): Response {
  const text = body === undefined ? "" : JSON.stringify(body);
  return {
    ok: status >= 200 && status < 300,
    status,
    text: async () => text,
    json: async () => body,
  } as unknown as Response;
}

function requestAt(
  mock: { mock: { calls: unknown[][] } },
  index = 0,
): { url: string; init: RequestInit } {
  const call = mock.mock.calls[index] as [string, RequestInit] | undefined;
  if (!call) throw new Error("fetch was not called");
  return { url: call[0], init: call[1] };
}

function detail(overrides: Partial<PluginDetail["plugin"]> = {}): PluginDetail {
  return {
    plugin: {
      id: "demo",
      name: "Demo",
      version: "1.2.0",
      digest: "a".repeat(64),
      previous_digest: null,
      enabled: true,
      settings: { greeting: "Hi" },
      settings_revision: 3,
      granted_scopes: ["ctx"],
      installed_at: 1_700_000_000,
      updated_at: 1_700_000_100,
      ...overrides,
    },
    manifest: {
      schema_version: "1",
      id: "demo",
      name: "Demo",
      version: "1.2.0",
      api_version: 2,
      runtime: { engine: "workerd", compatibility_date: "2026-11-01" },
      entry: { type: "worker", path: "worker/index.js" },
      jobs: [{ name: "sync" }],
    },
  };
}

function job(overrides: Partial<JobView> = {}): JobView {
  return {
    id: "job_1",
    plugin_id: "demo",
    name: "sync",
    status: "pending",
    attempts: 0,
    available_at: 1_700_000_000,
    last_error: null,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    ...overrides,
  };
}

function delivery(overrides: Partial<DeliveryView> = {}): DeliveryView {
  return {
    id: "pdl_1",
    plugin_id: "demo",
    event_id: "evt_1",
    event_type: "order.created",
    status: "dead",
    attempts: 5,
    available_at: 1_700_000_000,
    last_error: "timeout",
    created_at: 1_700_000_000,
    ...overrides,
  };
}

function log(overrides: Partial<LogView> = {}): LogView {
  return {
    id: 7,
    plugin_id: "demo",
    level: "info",
    message: "processed event",
    fields_json: '{"event":"evt_1"}',
    created_at: 1_700_000_000,
    ...overrides,
  };
}

beforeEach(() => {
  document.cookie = "fc_csrf=csrf-token";
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "fc_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT";
});

describe("plugin observability client", () => {
  it("reads one plugin detail with store and csrf headers", async () => {
    const mock = vi.fn(async () => jsonResponse(detail()));
    vi.stubGlobal("fetch", mock);

    const result = await getPlugin("store-1", "live/chat");
    expect(result.plugin.id).toBe("demo");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/live%2Fchat`);
    expect(init.method).toBeUndefined();
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("forwards an abort signal to a list endpoint", async () => {
    const mock = vi.fn(async () => jsonResponse([job()]));
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listJobs("store-1", "demo", controller.signal);

    expect(requestAt(mock).init.signal).toBe(controller.signal);
  });

  it("rolls back with a POST", async () => {
    const mock = vi.fn(async () => jsonResponse(detail().plugin));
    vi.stubGlobal("fetch", mock);

    await rollbackPlugin("store-1", "demo");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/demo/rollback`);
    expect(init.method).toBe("POST");
    expect((init.headers as Headers).get("x-csrf-token")).toBe("csrf-token");
  });

  it("uploads an upgrade archive as multipart without an explicit content type", async () => {
    const mock = vi.fn(async () => jsonResponse(detail().plugin));
    vi.stubGlobal("fetch", mock);
    const archive = new File(["bundle"], "demo.tar.gz", { type: "application/gzip" });

    await upgradePlugin("store-1", "demo", archive, true);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/demo/upgrade?accept_new_scopes=true`);
    expect(init.method).toBe("POST");
    const body = init.body as FormData;
    expect(body).toBeInstanceOf(FormData);
    expect(body.get("archive")).toBeInstanceOf(File);
    expect((init.headers as Headers).get("content-type")).toBeNull();
  });

  it("omits the accept flag when scopes do not change", async () => {
    const mock = vi.fn(async () => jsonResponse(detail().plugin));
    vi.stubGlobal("fetch", mock);
    const archive = new File(["bundle"], "demo.tar.gz");

    await upgradePlugin("store-1", "demo", archive);

    const { url } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/demo/upgrade`);
  });

  it("runs a job by name and accepts an empty 202 response", async () => {
    const mock = vi.fn(async () => jsonResponse(undefined, 202));
    vi.stubGlobal("fetch", mock);

    await expect(runJob("store-1", "demo", "sync nightly")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/demo/jobs/sync%20nightly/run`);
    expect(init.method).toBe("POST");
  });

  it("clears logs and accepts an empty 204 response", async () => {
    const mock = vi.fn(async () => jsonResponse(undefined, 204));
    vi.stubGlobal("fetch", mock);

    await expect(clearLogs("store-1", "demo")).resolves.toBeUndefined();

    expect(requestAt(mock).init.method).toBe("DELETE");
  });

  it("lists logs and deliveries", async () => {
    const mock = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse([log()]))
      .mockResolvedValueOnce(jsonResponse([delivery()]));
    vi.stubGlobal("fetch", mock);

    await expect(listLogs("store-1", "demo")).resolves.toEqual([log()]);
    await expect(listDeliveries("store-1", "demo")).resolves.toEqual([delivery()]);

    expect(requestAt(mock, 0).url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/demo/logs`);
    expect(requestAt(mock, 1).url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/demo/deliveries`);
  });

  it("replays a delivery", async () => {
    const mock = vi.fn(async () => jsonResponse(undefined, 202));
    vi.stubGlobal("fetch", mock);

    await replayDelivery("store-1", "demo", "pdl/1");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/demo/deliveries/pdl%2F1/replay`);
    expect(init.method).toBe("POST");
  });

  it("reads runtime status from the shared endpoint", async () => {
    const status: PluginRuntimeStatus = {
      running: true,
      generation: "gen_1",
      port: 8081,
      restarts: 2,
      last_error: null,
      uptime_seconds: 3600,
    };
    const mock = vi.fn(async () => jsonResponse(status));
    vi.stubGlobal("fetch", mock);

    await expect(runtimeStatus("store-1")).resolves.toEqual(status);

    expect(requestAt(mock).url).toBe(`${API_BASE_URL}/api/v1/admin/plugins/runtime`);
  });

  it("surfaces the server error message", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { code: "CONFLICT", message: "plugin runtime is disabled" } }, 409),
    );
    vi.stubGlobal("fetch", mock);

    await expect(rollbackPlugin("store-1", "demo")).rejects.toThrow("plugin runtime is disabled");
  });

  it("falls back to the http status when no error body is returned", async () => {
    const mock = vi.fn(async () => jsonResponse(undefined, 502));
    vi.stubGlobal("fetch", mock);

    await expect(listJobs("store-1", "demo")).rejects.toThrow("Plugin request failed (502)");
  });
});

describe("plugin observability helpers", () => {
  it("maps persisted statuses to admin tones", () => {
    expect(statusTone("done")).toBe("brand");
    expect(statusTone("completed")).toBe("brand");
    expect(statusTone("running")).toBe("info");
    expect(statusTone("pending")).toBe("warning");
    expect(statusTone("failed")).toBe("destructive");
    expect(statusTone("dead")).toBe("destructive");
    expect(statusTone("archived")).toBe("secondary");
  });

  it("detects statuses that still have work left", () => {
    expect(isActiveStatus("pending")).toBe(true);
    expect(isActiveStatus("Running")).toBe(true);
    expect(isActiveStatus("done")).toBe(false);
    expect(isActiveStatus("dead")).toBe(false);
  });

  it("parses only object-shaped log fields", () => {
    expect(parseLogFields('{"event":"evt_1"}')).toEqual({ event: "evt_1" });
    expect(parseLogFields(null)).toBeNull();
    expect(parseLogFields("")).toBeNull();
    expect(parseLogFields("[1,2]")).toBeNull();
    expect(parseLogFields("not json")).toBeNull();
  });
});
