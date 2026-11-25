import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { API_BASE_URL } from "@/lib/api";
import {
  bindCapability,
  contributionForKey,
  defaultConfigForSchema,
  groupCapabilities,
  initialConfig,
  initialContribution,
  isBoundProvider,
  listCapabilities,
  parseConfigObject,
  providerKey,
  unbindCapability,
  type CapabilitiesResponse,
  type CapabilityBinding,
  type CapabilityContribution,
  type JsonObject,
} from "@/lib/capabilities";

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
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

function contribution(overrides: Partial<CapabilityContribution> = {}): CapabilityContribution {
  return {
    plugin_id: "demo",
    plugin_name: "Demo plugin",
    plugin_version: "1.0.0",
    id: "chat",
    version: 1,
    title: "Live chat",
    config_schema: null,
    ...overrides,
  };
}

function binding(overrides: Partial<CapabilityBinding> = {}): CapabilityBinding {
  return {
    capability_id: "chat",
    version: 1,
    plugin_id: "demo",
    config: {},
    enabled: true,
    created_at: 1,
    updated_at: 2,
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

describe("capability client", () => {
  it("lists contributions and bindings with store and csrf headers", async () => {
    const response: CapabilitiesResponse = {
      contributions: [contribution()],
      bindings: [binding()],
    };
    const mock = vi.fn(async () => jsonResponse(response));
    vi.stubGlobal("fetch", mock);

    await expect(listCapabilities("store-1")).resolves.toEqual(response);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/capabilities`);
    expect(init.method).toBeUndefined();
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("forwards an abort signal", async () => {
    const mock = vi.fn(async () => jsonResponse({ contributions: [], bindings: [] }));
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listCapabilities("store-1", controller.signal);

    const { init } = requestAt(mock);
    expect(init.signal).toBe(controller.signal);
  });

  it("puts the binding payload as JSON", async () => {
    const result = binding({ version: 2, config: { greeting: "Hi" }, enabled: false });
    const mock = vi.fn(async () => jsonResponse(result));
    vi.stubGlobal("fetch", mock);

    await expect(
      bindCapability("store-1", "live chat", {
        plugin_id: "demo",
        version: 2,
        config: { greeting: "Hi" },
        enabled: false,
      }),
    ).resolves.toEqual(result);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/capabilities/live%20chat/binding`);
    expect(init.method).toBe("PUT");
    const headers = init.headers as Headers;
    expect(headers.get("content-type")).toBe("application/json");
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(JSON.parse(String(init.body))).toEqual({
      plugin_id: "demo",
      version: 2,
      config: { greeting: "Hi" },
      enabled: false,
    });
  });

  it("deletes a binding and accepts an empty 204 response", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(unbindCapability("store-1", "chat")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/capabilities/chat/binding`);
    expect(init.method).toBe("DELETE");
  });

  it("surfaces the server error message", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: "capability config must be a JSON object" }, 400),
    );
    vi.stubGlobal("fetch", mock);

    await expect(
      bindCapability("store-1", "chat", { plugin_id: "demo", version: 1, config: {} }),
    ).rejects.toThrow("capability config must be a JSON object");
  });

  it("falls back to the http status when no error body is returned", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 502));
    vi.stubGlobal("fetch", mock);

    await expect(listCapabilities("store-1")).rejects.toThrow("Capability request failed (502)");
  });
});

describe("capability config", () => {
  it("accepts only parsed JSON objects", () => {
    expect(parseConfigObject('{"greeting":"Hi"}')).toEqual({
      config: { greeting: "Hi" },
      error: null,
    });
    expect(parseConfigObject("{").error).toBe("Config must be valid JSON.");
    expect(parseConfigObject("[]").error).toBe("Config must be a JSON object.");
    expect(parseConfigObject("null").error).toBe("Config must be a JSON object.");
    expect(parseConfigObject('"text"').error).toBe("Config must be a JSON object.");
  });

  it("seeds config from a top-level schema default", () => {
    expect(
      defaultConfigForSchema({
        type: "object",
        default: { mode: "compact" },
        properties: { mode: { type: "string" } },
      }),
    ).toEqual({ mode: "compact" });
  });

  it("collects property defaults when the schema has no top-level default", () => {
    expect(
      defaultConfigForSchema({
        type: "object",
        properties: {
          greeting: { type: "string", default: "Hi" },
          count: { type: "number" },
          nested: { type: "object", default: { enabled: true } },
        },
      }),
    ).toEqual({ greeting: "Hi", nested: { enabled: true } });
  });

  it("returns an empty object for missing or non-object schemas", () => {
    expect(defaultConfigForSchema(null)).toEqual({});
    expect(defaultConfigForSchema({ type: "string" })).toEqual({});
  });
});

describe("capability grouping", () => {
  it("groups providers by id and prefers the binding's provider", () => {
    const response: CapabilitiesResponse = {
      contributions: [
        contribution({ plugin_id: "alpha", plugin_name: "Alpha", version: 1 }),
        contribution({
          plugin_id: "beta",
          plugin_name: "Beta",
          version: 2,
          title: "Live chat v2",
        }),
      ],
      bindings: [binding({ plugin_id: "beta", version: 2, config: { greeting: "Hi" } })],
    };

    const groups = groupCapabilities(response);
    expect(groups).toHaveLength(1);
    const group = groups[0];
    if (!group) throw new Error("expected a capability group");

    expect(group.title).toBe("Live chat v2");
    expect(group.contributions.map((entry) => entry.plugin_id)).toEqual(["alpha", "beta"]);
    expect(initialContribution(group)?.plugin_id).toBe("beta");
    expect(initialConfig(group, contributionForKey(group, "beta@2"))).toEqual({
      greeting: "Hi",
    });
    expect(initialConfig(group, contributionForKey(group, "alpha@1"))).toEqual({});
    expect(isBoundProvider(group.binding, contribution({ plugin_id: "beta", version: 2 }))).toBe(
      true,
    );
  });

  it("keeps bindings whose provider is no longer available", () => {
    const groups = groupCapabilities({
      contributions: [],
      bindings: [
        binding({ capability_id: "legacy", plugin_id: "removed", version: 7, config: {} }),
      ],
    });

    expect(groups).toHaveLength(1);
    const group = groups[0];
    if (!group) throw new Error("expected a capability group");

    expect(group.id).toBe("legacy");
    expect(group.title).toBe("legacy");
    expect(group.contributions).toEqual([]);
    expect(initialContribution(group)).toBeNull();
    expect(group.binding?.plugin_id).toBe("removed");
  });

  it("resolves provider keys back to contributions", () => {
    const group = groupCapabilities({
      contributions: [contribution({ plugin_id: "alpha", version: 3 })],
      bindings: [],
    })[0];
    if (!group) throw new Error("expected a capability group");

    expect(providerKey({ plugin_id: "alpha", version: 3 })).toBe("alpha@3");
    expect(contributionForKey(group, "alpha@3")?.version).toBe(3);
    expect(contributionForKey(group, "alpha@4")).toBeNull();
    expect(contributionForKey(null, "alpha@3")).toBeNull();
  });

  it("sorts groups by title and id", () => {
    const groups = groupCapabilities({
      contributions: [
        contribution({ id: "zeta", title: "Zeta" }),
        contribution({ id: "alpha", title: "Alpha" }),
        contribution({ id: "beta", title: "Alpha" }),
      ],
      bindings: [],
    });

    expect(groups.map((group) => group.id)).toEqual(["alpha", "beta", "zeta"]);
  });
});

describe("initial config", () => {
  it("uses the contribution schema when the binding belongs to another provider", () => {
    const schema: JsonObject = {
      type: "object",
      properties: { greeting: { type: "string", default: "Hello" } },
    };
    const group = groupCapabilities({
      contributions: [contribution({ plugin_id: "beta", config_schema: schema })],
      bindings: [binding({ plugin_id: "alpha", config: { greeting: "Old" } })],
    })[0];
    if (!group) throw new Error("expected a capability group");

    const selected = initialContribution(group);
    expect(selected?.plugin_id).toBe("beta");
    expect(initialConfig(group, selected)).toEqual({ greeting: "Hello" });
  });
});
