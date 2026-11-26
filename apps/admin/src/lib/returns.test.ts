import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StoreReturnRow } from "@/lib/returns";

import { API_BASE_URL } from "@/lib/api";
import {
  RETURN_STATUS_LABELS,
  RETURN_STATUSES,
  listStoreReturns,
  returnFilterOptions,
  returnStatusLabel,
  returnStatusTone,
} from "@/lib/returns";

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

const row: StoreReturnRow = {
  id: "return_1",
  order_id: "order_1",
  order_display_number: 1042,
  customer_email: "buyer@example.com",
  status: "requested",
  reason: "damaged",
  item_count: 2,
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

describe("store returns client", () => {
  it("lists returns with store, CSRF and pagination", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [row], pagination: { limit: 20, offset: 0, total: 1 } }),
    );
    vi.stubGlobal("fetch", mock);

    await expect(listStoreReturns("store-1", { limit: 20, offset: 0 })).resolves.toEqual({
      data: [row],
      pagination: { limit: 20, offset: 0, total: 1 },
    });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/returns?limit=20&offset=0`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("sends the status filter only when one is given", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [], pagination: { limit: 25, offset: 0, total: 0 } }),
    );
    vi.stubGlobal("fetch", mock);

    await listStoreReturns("store-1", { status: "partially_received" });
    await listStoreReturns("store-1");

    expect(requestAt(mock, 0).url).toBe(
      `${API_BASE_URL}/api/v1/admin/returns?limit=25&offset=0&status=partially_received`,
    );
    expect(requestAt(mock, 1).url).toBe(`${API_BASE_URL}/api/v1/admin/returns?limit=25&offset=0`);
  });

  it("forwards the abort signal", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [], pagination: { limit: 25, offset: 0, total: 0 } }),
    );
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listStoreReturns("store-1", {}, controller.signal);

    expect(requestAt(mock).init.signal).toBe(controller.signal);
  });

  it("surfaces API error envelopes", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { message: "limit must be between 1 and 100" } }, 400),
    );
    vi.stubGlobal("fetch", mock);

    await expect(listStoreReturns("store-1", { limit: 500 })).rejects.toThrow(
      "limit must be between 1 and 100",
    );
  });
});

describe("return status helpers", () => {
  it("labels every status", () => {
    expect(returnStatusLabel("draft")).toBe("Draft");
    expect(returnStatusLabel("partially_received")).toBe("Partially received");
    expect(returnStatusLabel("written_off")).toBe("Written off");
    expect(returnStatusLabel("something_new")).toBe("something_new");
  });

  it("maps every status to a badge tone", () => {
    for (const status of RETURN_STATUSES) {
      expect(returnStatusTone(status)).toBeTruthy();
    }
    expect(returnStatusTone("received")).toBe("brand");
    expect(returnStatusTone("canceled")).toBe("destructive");
    expect(returnStatusTone("unknown")).toBe("secondary");
  });

  it("builds the filter options from the status list", () => {
    const options = returnFilterOptions();
    expect(options[0]).toEqual({ value: "all", label: "All" });
    expect(options).toHaveLength(RETURN_STATUSES.length + 1);
    expect(options).toContainEqual({
      value: "requested",
      label: RETURN_STATUS_LABELS.requested,
    });
  });
});
