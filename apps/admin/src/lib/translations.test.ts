import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Translation } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  collectTranslationFields,
  deleteTranslation,
  hasTranslatedValue,
  isTranslationNotFound,
  listTranslations,
  mergeTranslationFields,
  translationCompleteness,
  translationKey,
  translationText,
  upsertTranslation,
} from "@/lib/translations";

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

function translation(overrides: Partial<Translation> = {}): Translation {
  return {
    id: "tr_1",
    resource_type: "product",
    resource_id: "prd_1",
    locale: "fr-FR",
    fields: { title: "Chaise", description: "Bois" },
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

describe("translation client", () => {
  it("lists every locale of a resource", async () => {
    const mock = vi.fn(async () => jsonResponse([translation()]));
    vi.stubGlobal("fetch", mock);

    await expect(
      listTranslations("store-1", { resource_type: "product", resource_id: "prd_1" }),
    ).resolves.toEqual([translation()]);

    const { url, init } = requestAt(mock);
    expect(url).toBe(
      `${API_BASE_URL}/api/v1/admin/content/translations?resource_type=product&resource_id=prd_1`,
    );
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("scopes the list to one locale when asked", async () => {
    const mock = vi.fn(async () => jsonResponse([translation()]));
    vi.stubGlobal("fetch", mock);

    await listTranslations("store-1", {
      resource_type: "page",
      resource_id: "page 1",
      locale: "fr-FR",
    });

    const params = new URL(requestAt(mock).url).searchParams;
    expect(params.get("resource_type")).toBe("page");
    expect(params.get("resource_id")).toBe("page 1");
    expect(params.get("locale")).toBe("fr-FR");
  });

  it("upserts a translation with the JSON fields body", async () => {
    const mock = vi.fn(async () => jsonResponse(translation()));
    vi.stubGlobal("fetch", mock);

    await upsertTranslation(
      "store-1",
      { resource_type: "product", resource_id: "prd_1", locale: "fr-FR" },
      { fields: { title: "Chaise" } },
    );

    const { url, init } = requestAt(mock);
    expect(url).toContain("/api/v1/admin/content/translations?");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body as string)).toEqual({ fields: { title: "Chaise" } });
  });

  it("deletes a translation and returns void", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(
      deleteTranslation("store-1", {
        resource_type: "product",
        resource_id: "prd_1",
        locale: "fr-FR",
      }),
    ).resolves.toBeUndefined();

    const { init } = requestAt(mock);
    expect(init.method).toBe("DELETE");
  });

  it("forwards the abort signal", async () => {
    const mock = vi.fn(async () => jsonResponse([]));
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listTranslations(
      "store-1",
      { resource_type: "product", resource_id: "prd_1" },
      controller.signal,
    );

    expect(requestAt(mock).init.signal).toBe(controller.signal);
  });

  it("flags a missing translation as not found rather than a failure", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { code: "NOT_FOUND", message: "translation not found" } }, 404),
    );
    vi.stubGlobal("fetch", mock);

    const cause = await listTranslations("store-1", {
      resource_type: "product",
      resource_id: "prd_1",
      locale: "fr-FR",
    }).catch((error: unknown) => error);

    expect(isTranslationNotFound(cause)).toBe(true);
    expect(cause).toBeInstanceOf(Error);
    expect((cause as Error).message).toBe("translation not found");
  });

  it("surfaces other API failures with their status", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { message: "Fields are too large." } }, 400),
    );
    vi.stubGlobal("fetch", mock);

    const cause = await upsertTranslation(
      "store-1",
      { resource_type: "product", resource_id: "prd_1", locale: "fr-FR" },
      { fields: { title: "x" } },
    ).catch((error: unknown) => error);

    expect(isTranslationNotFound(cause)).toBe(false);
    expect((cause as Error).message).toBe("Fields are too large.");
  });
});

describe("translation helpers", () => {
  it("treats blank strings and null as untranslated", () => {
    expect(hasTranslatedValue("Chaise")).toBe(true);
    expect(hasTranslatedValue("   ")).toBe(false);
    expect(hasTranslatedValue("")).toBe(false);
    expect(hasTranslatedValue(null)).toBe(false);
    expect(hasTranslatedValue(undefined)).toBe(false);
    expect(hasTranslatedValue(0)).toBe(true);
  });

  it("renders scalar values as editable text", () => {
    expect(translationText("Chaise")).toBe("Chaise");
    expect(translationText(12)).toBe("12");
    expect(translationText({ nested: true })).toBe("");
    expect(translationText(null)).toBe("");
  });

  it("overlays translated fields and keeps source fallbacks", () => {
    const source = { title: "Chair", description: "Wood", handle: "chair" };
    const merged = mergeTranslationFields(source, {
      fields: { title: "Chaise", description: "   " },
    });

    expect(merged).toEqual({ title: "Chaise", description: "Wood", handle: "chair" });
    // The source object is not mutated.
    expect(source.title).toBe("Chair");
  });

  it("restricts the merge to the requested keys", () => {
    const merged = mergeTranslationFields(
      { title: "Chair", handle: "chair" },
      { fields: { title: "Chaise", handle: "chaise" } },
      ["title"],
    );
    expect(merged).toEqual({ title: "Chaise", handle: "chair" });
  });

  it("handles a missing translation row", () => {
    expect(mergeTranslationFields({ title: "Chair" }, null)).toEqual({ title: "Chair" });
  });

  it("reports field completeness in input order", () => {
    const completeness = translationCompleteness(
      { fields: { title: "Chaise", description: " " } },
      ["title", "description", "seo_title"],
    );
    expect(completeness).toEqual({
      translated: 1,
      total: 3,
      missing: ["description", "seo_title"],
    });
    expect(translationCompleteness(null, ["title"])).toEqual({
      translated: 0,
      total: 1,
      missing: ["title"],
    });
  });

  it("omits blank fields from the stored translation", () => {
    expect(
      collectTranslationFields({ title: " Chaise ", description: "  ", seo_title: "" }, [
        "title",
        "description",
        "seo_title",
      ]),
    ).toEqual({ title: "Chaise" });
  });

  it("builds distinct keys per resource and locale", () => {
    expect(translationKey("page", "page_1", "fr-FR")).not.toBe(
      translationKey("page", "page_1", "de-DE"),
    );
    expect(translationKey("page", "page_1", "fr-FR")).not.toBe(
      translationKey("product", "page_1", "fr-FR"),
    );
  });
});
