import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { API_BASE_URL } from "@/lib/api";
import {
  createPriceList,
  deletePriceList,
  listPriceListPrices,
  listPriceLists,
  priceListEnded,
  priceListTypeHint,
  pricesForList,
  updatePriceList,
  type PriceListInput,
  type PriceListPrice,
} from "@/lib/price-lists";

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

function priceListInput(overrides: Partial<PriceListInput> = {}): PriceListInput {
  return {
    title: "Wholesale",
    description: null,
    price_type: "sale",
    status: "draft",
    ends_at: null,
    ...overrides,
  };
}

function price(overrides: Partial<PriceListPrice> = {}): PriceListPrice {
  return {
    id: "price-1",
    price_list_id: "pl_1",
    variant_id: "variant-1",
    currency_code: "USD",
    amount: 1500,
    min_quantity: 1,
    max_quantity: null,
    customer_group_id: null,
    sales_channel_id: null,
    starts_at: null,
    ends_at: null,
    metadata: {},
    created_at: 1,
    updated_at: 1,
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

describe("price list client", () => {
  it("lists price lists with store, csrf and pagination", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [], pagination: { limit: 20, offset: 0, total: 0 } }),
    );
    vi.stubGlobal("fetch", mock);

    await listPriceLists("store-1", { limit: 20, offset: 0 });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/price-lists?limit=20&offset=0`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("omits the query string when no filters are passed", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [], pagination: { limit: 0, offset: 0, total: 0 } }),
    );
    vi.stubGlobal("fetch", mock);

    await listPriceLists("store-1");

    expect(requestAt(mock).url).toBe(`${API_BASE_URL}/api/v1/admin/price-lists`);
  });

  it("forwards an abort signal", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [], pagination: { limit: 0, offset: 0, total: 0 } }),
    );
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listPriceLists("store-1", {}, controller.signal);

    expect(requestAt(mock).init.signal).toBe(controller.signal);
  });

  it("creates a price list with a JSON body", async () => {
    const mock = vi.fn(async () => jsonResponse({ id: "pl_1" }, 201));
    vi.stubGlobal("fetch", mock);

    await createPriceList(
      "store-1",
      priceListInput({ title: "Spring sale", price_type: "override", ends_at: 1_800 }),
    );

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/price-lists`);
    expect(init.method).toBe("POST");
    expect((init.headers as Headers).get("content-type")).toBe("application/json");
    expect(JSON.parse(String(init.body))).toEqual({
      title: "Spring sale",
      description: null,
      price_type: "override",
      status: "draft",
      ends_at: 1_800,
    });
  });

  it("patches only the provided fields", async () => {
    const mock = vi.fn(async () => jsonResponse({ id: "pl 1" }));
    vi.stubGlobal("fetch", mock);

    await updatePriceList("store-1", "pl 1", { status: "active" });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/price-lists/pl%201`);
    expect(init.method).toBe("PATCH");
    expect(JSON.parse(String(init.body))).toEqual({ status: "active" });
  });

  it("deletes a price list and accepts an empty 204", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(deletePriceList("store-1", "pl_1")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/price-lists/pl_1`);
    expect(init.method).toBe("DELETE");
  });

  it("lists prices filtered by price list id", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [price()], pagination: { limit: 50, offset: 0, total: 1 } }),
    );
    vi.stubGlobal("fetch", mock);

    const result = await listPriceListPrices("store-1", "pl_1", { limit: 50 });

    const { url } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/prices?price_list_id=pl_1&limit=50`);
    expect(result.data[0]?.price_list_id).toBe("pl_1");
  });

  it("surfaces the API error envelope", async () => {
    const mock = vi.fn(async () =>
      jsonResponse(
        { error: { code: "VALIDATION_ERROR", message: "price list title must not be empty" } },
        400,
      ),
    );
    vi.stubGlobal("fetch", mock);

    await expect(createPriceList("store-1", priceListInput({ title: "" }))).rejects.toThrow(
      "price list title must not be empty",
    );
  });

  it("falls back to the http status without an error body", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 503));
    vi.stubGlobal("fetch", mock);

    await expect(listPriceLists("store-1")).rejects.toThrow("Price list request failed (503)");
  });
});

describe("price list helpers", () => {
  it("explains each price type", () => {
    expect(priceListTypeHint("sale")).toContain("discounts");
    expect(priceListTypeHint("override")).toContain("replace");
  });

  it("detects ended lists", () => {
    expect(priceListEnded(null, 100)).toBe(false);
    expect(priceListEnded(100, 100)).toBe(true);
    expect(priceListEnded(200, 100)).toBe(false);
  });

  it("keeps only the rows of one list", () => {
    const rows = [price(), price({ id: "price-2", price_list_id: null })];
    expect(pricesForList(rows, "pl_1").map((row) => row.id)).toEqual(["price-1"]);
  });
});
