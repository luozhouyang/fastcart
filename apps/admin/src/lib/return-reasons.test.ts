import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReturnReason } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  createReturnReason,
  deleteReturnReason,
  listReturnReasons,
  returnReasonInputError,
  updateReturnReason,
} from "@/lib/return-reasons";

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

const reason: ReturnReason = {
  id: "rreason_1",
  value: "damaged",
  label: "Arrived damaged",
  description: null,
  metadata: {},
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

describe("return reason client", () => {
  it("lists reasons with store and CSRF headers", async () => {
    const mock = vi.fn(async () => jsonResponse([reason]));
    vi.stubGlobal("fetch", mock);

    await expect(listReturnReasons("store-1")).resolves.toEqual([reason]);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/return-reasons`);
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("creates a reason", async () => {
    const mock = vi.fn(async () => jsonResponse(reason, 201));
    vi.stubGlobal("fetch", mock);

    await createReturnReason("store-1", { value: "damaged", label: "Arrived damaged" });

    const { init } = requestAt(mock);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({
      value: "damaged",
      label: "Arrived damaged",
    });
  });

  it("patches a reason by encoded id", async () => {
    const mock = vi.fn(async () => jsonResponse({ ...reason, label: "Damaged" }));
    vi.stubGlobal("fetch", mock);

    await updateReturnReason("store-1", "rreason 1", { label: "Damaged", description: null });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/return-reasons/rreason%201`);
    expect(init.method).toBe("PATCH");
    expect(JSON.parse(init.body as string)).toEqual({ label: "Damaged", description: null });
  });

  it("deletes a reason and returns void", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(deleteReturnReason("store-1", "rreason_1")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/return-reasons/rreason_1`);
    expect(init.method).toBe("DELETE");
  });

  it("surfaces API error envelopes", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { message: "A return reason with this value exists." } }, 409),
    );
    vi.stubGlobal("fetch", mock);

    await expect(
      createReturnReason("store-1", { value: "damaged", label: "Arrived damaged" }),
    ).rejects.toThrow("with this value exists");
  });
});

describe("return reason validation", () => {
  it("requires a value and a label", () => {
    expect(returnReasonInputError({ value: "", label: "Damaged" })).toMatch(/value/);
    expect(returnReasonInputError({ value: "damaged", label: " " })).toMatch(/label/);
    expect(returnReasonInputError({ value: "damaged", label: "Arrived damaged" })).toBeNull();
  });

  it("bounds the lengths the API enforces", () => {
    expect(returnReasonInputError({ value: "v".repeat(121), label: "Damaged" })).toMatch(/120/);
    expect(returnReasonInputError({ value: "damaged", label: "l".repeat(201) })).toMatch(/200/);
  });
});
