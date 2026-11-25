import type { ThemeEditorManifest } from "@fastcart/shared-types";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { API_BASE_URL } from "@/lib/api";
import {
  listPluginComponents,
  mergePluginComponents,
  type PluginComponentContribution,
} from "@/lib/plugin-components";

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

function contribution(
  overrides: Partial<PluginComponentContribution> = {},
): PluginComponentContribution {
  return {
    plugin_id: "chat-widget",
    plugin_name: "Chat Widget",
    plugin_version: "1.0.0",
    digest: "0".repeat(64),
    id: "livechat-button",
    title: "Live chat button",
    category: null,
    editor_manifest: null,
    entry:
      "/_fastcart/extensions/chat-widget/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/components/livechat.js",
    requires: [{ id: "livechat", version: 1, optional: false }],
    ...overrides,
  };
}

function baseManifest(): ThemeEditorManifest {
  return {
    schema_version: 1,
    categories: {
      hero: { title: "Hero", components: ["Hero"] },
      commerce: { title: "Commerce", components: ["ProductGrid"] },
    },
    components: {
      Hero: {
        title: "Hero banner",
        category: "hero",
        fields: { heading: { type: "text" } },
        defaultProps: { heading: "Hello" },
        slots: [],
      },
      ProductGrid: { title: "Product grid", category: "commerce", slots: [] },
    },
    root: { fields: { page_title: { type: "text", label: "Page title" } } },
    zones: { header: { title: "Header", allowed_components: ["Hero"] } },
  };
}

beforeEach(() => {
  document.cookie = "fc_csrf=csrf-token";
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "fc_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT";
});

describe("plugin component client", () => {
  it("lists components with store and csrf headers", async () => {
    const components = [contribution()];
    const mock = vi.fn(async () => jsonResponse({ components }));
    vi.stubGlobal("fetch", mock);

    await expect(listPluginComponents("store-1")).resolves.toEqual(components);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/components`);
    expect(init.method).toBeUndefined();
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("forwards an abort signal", async () => {
    const mock = vi.fn(async () => jsonResponse({ components: [] }));
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listPluginComponents("store-1", controller.signal);

    const { init } = requestAt(mock);
    expect(init.signal).toBe(controller.signal);
  });

  it("surfaces the server error message", async () => {
    const mock = vi.fn(async () =>
      jsonResponse(
        {
          error: {
            code: "FORBIDDEN",
            message: "You do not have permission to read components.",
          },
        },
        403,
      ),
    );
    vi.stubGlobal("fetch", mock);

    await expect(listPluginComponents("store-1")).rejects.toThrow(
      "You do not have permission to read components.",
    );
  });

  it("falls back to the http status when no error body is returned", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 502));
    vi.stubGlobal("fetch", mock);

    await expect(listPluginComponents("store-1")).rejects.toThrow(
      "Plugin component request failed (502)",
    );
  });
});

describe("plugin component merge", () => {
  it("adds a component described by a full editor manifest", () => {
    const base = baseManifest();
    const merged = mergePluginComponents(base, [
      contribution({
        editor_manifest: {
          title: "Live chat",
          description: "Answers shopper questions.",
          icon: "message-circle",
          category: "commerce",
          fields: {
            greeting: {
              type: "text",
              label: "Greeting",
              options: [{ value: "hi", label: "Hi" }],
            },
          },
          defaultProps: { greeting: "Hi" },
          slots: [{ name: "content", title: "Content", allowed_components: ["Hero"] }],
        },
      }),
    ]);

    expect(merged.categories?.commerce?.components).toEqual(["ProductGrid", "livechat-button"]);
    expect(merged.components?.["livechat-button"]).toEqual({
      title: "Live chat",
      category: "commerce",
      description: "Answers shopper questions.",
      icon: "message-circle",
      fields: {
        greeting: {
          type: "text",
          label: "Greeting",
          options: [{ value: "hi", label: "Hi" }],
        },
      },
      defaultProps: { greeting: "Hi" },
      slots: [{ name: "content", title: "Content", allowed_components: ["Hero"] }],
    });
  });

  it("falls back to the contribution title", () => {
    const merged = mergePluginComponents(baseManifest(), [
      contribution({
        title: "Inline chat",
        category: "hero",
        editor_manifest: { description: 7, icon: "chat" },
      }),
    ]);

    const component = merged.components?.["livechat-button"];
    expect(component?.title).toBe("Inline chat");
    expect(component?.description).toBeUndefined();
    expect(component?.icon).toBe("chat");
    expect(merged.categories?.hero?.components).toEqual(["Hero", "livechat-button"]);
  });

  it("uses a category declared by the contribution", () => {
    const merged = mergePluginComponents(baseManifest(), [contribution({ category: "commerce" })]);

    expect(merged.categories?.commerce?.components).toEqual(["ProductGrid", "livechat-button"]);
    expect(merged.components?.["livechat-button"]?.category).toBe("commerce");
  });

  it("creates a plugin category for unknown categories", () => {
    const merged = mergePluginComponents(baseManifest(), [contribution({ category: "merch" })]);

    expect(merged.categories?.plugins).toEqual({
      title: "Plugin components",
      components: ["livechat-button"],
    });
    expect(merged.components?.["livechat-button"]?.category).toBe("plugins");
    expect(Object.keys(merged.categories ?? {})).toEqual(["hero", "commerce", "plugins"]);
  });

  it("reuses an existing plugin category and its title", () => {
    const base = baseManifest();
    base.categories = {
      ...base.categories,
      plugins: { title: "Extensions", components: ["Widget"] },
    };

    const merged = mergePluginComponents(base, [contribution({ category: "mystery" })]);

    expect(merged.categories?.plugins).toEqual({
      title: "Extensions",
      components: ["Widget", "livechat-button"],
    });
  });

  it("skips ids that collide with built-in components", () => {
    const base = baseManifest();
    const merged = mergePluginComponents(base, [
      contribution({ id: "Hero", title: "Impostor", editor_manifest: { title: "Impostor" } }),
    ]);

    expect(merged.components?.Hero?.title).toBe("Hero banner");
    expect(merged.categories?.plugins).toBeUndefined();
    expect(merged).toEqual(base);
  });

  it("skips contributions without an id", () => {
    const base = baseManifest();
    expect(mergePluginComponents(base, [contribution({ id: "" })])).toEqual(base);
  });

  it("keeps the first component for duplicate ids", () => {
    const merged = mergePluginComponents(baseManifest(), [
      contribution({ id: "chat", title: "First", category: "hero" }),
      contribution({ id: "chat", title: "Second", category: "commerce" }),
    ]);

    expect(merged.components?.chat?.title).toBe("First");
    expect(merged.categories?.hero?.components).toEqual(["Hero", "chat"]);
    expect(merged.categories?.commerce?.components).toEqual(["ProductGrid"]);
  });

  it("sanitizes malformed fields and slots", () => {
    const merged = mergePluginComponents(baseManifest(), [
      contribution({
        editor_manifest: {
          title: "Sanitized",
          fields: {
            good: { type: "text", options: [{ value: "a", label: "A" }] },
            bad_type: { type: 42, label: "Bad" },
            bad_options: { type: "select", options: "none" },
            not_object: "text",
          },
          slots: [
            { name: "items", title: 7, allowed_components: ["Hero", 2] },
            { name: 7, allowed_components: ["Hero"] },
            { name: "content", title: "Content", allowed_components: ["Hero", "ProductGrid"] },
            "nope",
          ],
          defaultProps: { nested: { enabled: true } },
        },
      }),
    ]);

    const component = merged.components?.["livechat-button"];
    expect(Object.keys(component?.fields ?? {})).toEqual(["good", "bad_options"]);
    expect(component?.fields?.good).toEqual({
      type: "text",
      options: [{ value: "a", label: "A" }],
    });
    expect(component?.fields?.bad_options).toEqual({ type: "select" });
    expect(component?.slots).toEqual([
      { name: "items" },
      { name: "content", title: "Content", allowed_components: ["Hero", "ProductGrid"] },
    ]);
    expect(component?.defaultProps).toEqual({ nested: { enabled: true } });
  });

  it("deep clones default props", () => {
    const defaults = { nested: { enabled: true } };
    const merged = mergePluginComponents(baseManifest(), [
      contribution({ editor_manifest: { title: "Chat", defaultProps: defaults } }),
    ]);

    const result = merged.components?.["livechat-button"]?.defaultProps;
    expect(result).toEqual(defaults);
    expect(result).not.toBe(defaults);
    const resultDefaults = result as { nested: { enabled: boolean } };
    expect(resultDefaults.nested).not.toBe(defaults.nested);
    resultDefaults.nested.enabled = false;
    expect(defaults.nested.enabled).toBe(true);
  });

  it("does not mutate the base manifest", () => {
    const base = baseManifest();
    const snapshot = JSON.stringify(base);

    const merged = mergePluginComponents(base, [contribution({ category: "hero" })]);

    expect(JSON.stringify(base)).toBe(snapshot);
    expect(merged.categories?.hero?.components).not.toBe(base.categories?.hero?.components);
    expect(merged.components).not.toBe(base.components);
  });

  it("returns an equivalent manifest for empty input", () => {
    const base = baseManifest();
    expect(mergePluginComponents(base, [])).toEqual(base);
  });
});
