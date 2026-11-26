import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Asset } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  assetUrl,
  deleteAsset,
  dimensionLabel,
  formatBytes,
  isAcceptedFile,
  isPreviewable,
  listAssets,
  uploadAsset,
} from "@/lib/media";

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

function asset(overrides: Partial<Asset> = {}): Asset {
  return {
    id: "asset_1",
    filename: "hero.png",
    content_type: "image/png",
    size_bytes: 2048,
    storage_key: "assets/store/asset_1/hero.png",
    alt: null,
    width: 1600,
    height: 900,
    created_by: null,
    created_at: 1,
    deleted_at: null,
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

describe("media client", () => {
  it("lists assets with a bounded page and search term", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [asset()], pagination: { limit: 60, offset: 0, total: 1 } }),
    );
    vi.stubGlobal("fetch", mock);

    const result = await listAssets("store-1", { search: "  hero " });
    expect(result.data).toHaveLength(1);

    const { url, init } = requestAt(mock);
    expect(url).toContain(`${API_BASE_URL}/api/v1/admin/assets?`);
    expect(url).toContain("search=hero");
    expect(url).toContain("limit=60");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("uploads multipart form data without a JSON content type", async () => {
    const mock = vi.fn(async () => jsonResponse(asset(), 201));
    vi.stubGlobal("fetch", mock);
    const file = new File([new Uint8Array([1, 2, 3])], "hero.png", { type: "image/png" });

    await uploadAsset("store-1", file, { alt: "  Hero image  ", width: 1600 });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/assets`);
    expect(init.method).toBe("POST");
    expect(init.body).toBeInstanceOf(FormData);
    const form = init.body as FormData;
    expect((form.get("file") as File).name).toBe("hero.png");
    expect(form.get("alt")).toBe("Hero image");
    expect(form.get("width")).toBe("1600");
    expect(form.get("height")).toBeNull();
    expect((init.headers as Headers).get("content-type")).toBeNull();
  });

  it("skips an empty alt value", async () => {
    const mock = vi.fn(async () => jsonResponse(asset(), 201));
    vi.stubGlobal("fetch", mock);

    await uploadAsset("store-1", new Blob([new Uint8Array([1])]), { alt: "   " });

    const form = requestAt(mock).init.body as FormData;
    expect(form.get("alt")).toBeNull();
  });

  it("deletes an asset by encoded id and returns void", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(deleteAsset("store-1", "asset/1")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/assets/asset%2F1`);
    expect(init.method).toBe("DELETE");
  });

  it("surfaces the API error message", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { code: "validation", message: "unsupported file type" } }, 422),
    );
    vi.stubGlobal("fetch", mock);

    await expect(uploadAsset("store-1", new Blob([new Uint8Array([1])]))).rejects.toThrow(
      "unsupported file type",
    );
  });

  it("builds storefront asset URLs", () => {
    expect(assetUrl("asset_1")).toBe("/api/v1/store/assets/asset_1");
    expect(assetUrl("a b")).toBe("/api/v1/store/assets/a%20b");
  });
});

describe("media helpers", () => {
  it("formats byte sizes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(900)).toBe("900 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(1536 * 1024)).toBe("1.5 MB");
    expect(formatBytes(120 * 1024 * 1024)).toBe("120 MB");
    expect(formatBytes(-1)).toBe("0 B");
  });

  it("renders dimensions only when both sides are known", () => {
    expect(dimensionLabel({ width: 1600, height: 900 })).toBe("1600 × 900");
    expect(dimensionLabel({ width: null, height: 900 })).toBeNull();
    expect(dimensionLabel({ width: 1600, height: null })).toBeNull();
  });

  it("previews only image content types", () => {
    expect(isPreviewable({ content_type: "image/png" })).toBe(true);
    expect(isPreviewable({ content_type: "image/webp" })).toBe(true);
    expect(isPreviewable({ content_type: "application/pdf" })).toBe(false);
    expect(isPreviewable({ content_type: "video/mp4" })).toBe(false);
  });

  it("accepts supported files by type or extension", () => {
    expect(isAcceptedFile({ type: "image/png", name: "hero.png" })).toBe(true);
    expect(isAcceptedFile({ type: "", name: "photo.JPEG" })).toBe(true);
    expect(isAcceptedFile({ type: "", name: "report.pdf" })).toBe(true);
    expect(isAcceptedFile({ type: "text/plain", name: "notes.txt" })).toBe(false);
  });
});
