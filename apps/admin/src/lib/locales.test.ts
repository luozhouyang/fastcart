import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StoreLocale } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  addStoreLocale,
  listStoreLocales,
  localeInputError,
  removeStoreLocale,
  setDefaultStoreLocale,
  updateStoreLocale,
} from "@/lib/locales";

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

const locale: StoreLocale = {
  store_id: "store_1",
  locale: "fr-FR",
  is_default: false,
  label: "Français",
  direction: "ltr",
  created_at: 1,
  updated_at: 2,
};

beforeEach(() => {
  document.cookie = "fc_csrf=csrf-token";
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "fc_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT";
});

describe("locale client", () => {
  it("lists locales for the store", async () => {
    const mock = vi.fn(async () => jsonResponse([locale]));
    vi.stubGlobal("fetch", mock);

    await expect(listStoreLocales("store-1")).resolves.toEqual([locale]);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/platform/stores/store-1/locales`);
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("adds a locale", async () => {
    const mock = vi.fn(async () => jsonResponse(locale, 201));
    vi.stubGlobal("fetch", mock);

    await addStoreLocale("store-1", { locale: "fr-FR" });

    const { init } = requestAt(mock);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ locale: "fr-FR" });
  });

  it("promotes a locale to default", async () => {
    const mock = vi.fn(async () => jsonResponse({ ...locale, is_default: true }));
    vi.stubGlobal("fetch", mock);

    await setDefaultStoreLocale("store-1", "fr-FR");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/platform/stores/store-1/locales/fr-FR/default`);
    expect(init.method).toBe("POST");
  });

  it("patches and removes by encoded locale", async () => {
    const mock = vi.fn(async () => jsonResponse(locale));
    vi.stubGlobal("fetch", mock);

    await updateStoreLocale("store-1", "zh Hant", { direction: "rtl" });
    const first = requestAt(mock, 0);
    expect(first.url).toBe(`${API_BASE_URL}/api/v1/platform/stores/store-1/locales/zh%20Hant`);
    expect(first.init.method).toBe("PATCH");
    expect(JSON.parse(first.init.body as string)).toEqual({ direction: "rtl" });

    vi.stubGlobal(
      "fetch",
      vi.fn(async () => jsonResponse(null, 204)),
    );
    await expect(removeStoreLocale("store-1", "fr-FR")).resolves.toBeUndefined();
  });

  it("surfaces API error envelopes", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { message: "The default locale cannot be removed." } }, 409),
    );
    vi.stubGlobal("fetch", mock);

    await expect(removeStoreLocale("store-1", "en-US")).rejects.toThrow(/cannot be removed/);
  });
});

describe("locale validation", () => {
  it("accepts BCP-47 tags and normalizes underscores", () => {
    expect(localeInputError("fr-FR")).toBeNull();
    expect(localeInputError("pt_BR")).toBeNull();
    expect(localeInputError("zh-Hant-TW")).toBeNull();
  });

  it("rejects malformed tags", () => {
    expect(localeInputError("")).toMatch(/Enter a locale/);
    expect(localeInputError("fr--FR")).toMatch(/BCP-47/);
    expect(localeInputError("fr FR")).toMatch(/BCP-47/);
    expect(localeInputError("-FR")).toMatch(/BCP-47/);
    expect(localeInputError("a".repeat(36))).toMatch(/35/);
  });
});
