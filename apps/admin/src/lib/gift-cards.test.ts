import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { GiftCard } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  giftCardInputError,
  giftCardStatusLabel,
  issueGiftCard,
  listGiftCards,
  remainingFraction,
  setGiftCardEnabled,
} from "@/lib/gift-cards";

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

const card: GiftCard = {
  id: "gift_1",
  code: "ABCD1234EFGH5678",
  currency_code: "USD",
  initial_amount_minor: 5_000,
  balance_minor: 2_500,
  status: "active",
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

describe("gift card client", () => {
  it("lists cards with store, CSRF and pagination", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [card], pagination: { limit: 25, offset: 0, total: 1 } }),
    );
    vi.stubGlobal("fetch", mock);

    await expect(listGiftCards("store-1", { limit: 10, offset: 20 })).resolves.toEqual({
      data: [card],
      pagination: { limit: 25, offset: 0, total: 1 },
    });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/gift-cards?limit=10&offset=20`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("issues a card with the JSON body", async () => {
    const mock = vi.fn(async () => jsonResponse(card, 201));
    vi.stubGlobal("fetch", mock);

    await issueGiftCard("store-1", { currency_code: "USD", amount_minor: 5_000 });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/gift-cards`);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({
      currency_code: "USD",
      amount_minor: 5_000,
    });
  });

  it("enables and disables by encoded id", async () => {
    const mock = vi.fn(async () => jsonResponse(card));
    vi.stubGlobal("fetch", mock);

    await setGiftCardEnabled("store-1", "gift 1", false);
    await setGiftCardEnabled("store-1", "gift 1", true);

    const first = requestAt(mock, 0);
    expect(first.url).toBe(`${API_BASE_URL}/api/v1/admin/gift-cards/gift%201/disable`);
    expect(first.init.method).toBe("POST");
    const second = requestAt(mock, 1);
    expect(second.url).toBe(`${API_BASE_URL}/api/v1/admin/gift-cards/gift%201/enable`);
  });

  it("surfaces API error envelopes", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { message: "gift card code X already exists" } }, 409),
    );
    vi.stubGlobal("fetch", mock);

    await expect(
      issueGiftCard("store-1", { currency_code: "USD", amount_minor: 100, code: "X" }),
    ).rejects.toThrow("already exists");
  });
});

describe("gift card helpers", () => {
  it("computes the remaining fraction with clamping", () => {
    expect(remainingFraction({ initial_amount_minor: 100, balance_minor: 25 })).toBe(0.25);
    expect(remainingFraction({ initial_amount_minor: 0, balance_minor: 25 })).toBe(0);
    expect(remainingFraction({ initial_amount_minor: 100, balance_minor: 250 })).toBe(1);
    expect(remainingFraction({ initial_amount_minor: 100, balance_minor: -10 })).toBe(0);
  });

  it("labels statuses", () => {
    expect(giftCardStatusLabel("active")).toBe("Active");
    expect(giftCardStatusLabel("disabled")).toBe("Disabled");
  });

  it("validates the issue form", () => {
    expect(giftCardInputError({ amount: "50", currency: "USD", code: "" })).toBeNull();
    expect(giftCardInputError({ amount: "0", currency: "USD", code: "" })).toMatch(/greater/);
    expect(giftCardInputError({ amount: "0.001", currency: "USD", code: "" })).toMatch(/cent/);
    expect(giftCardInputError({ amount: "5", currency: "US", code: "" })).toMatch(/three-letter/);
    expect(giftCardInputError({ amount: "5", currency: "USD", code: "x".repeat(65) })).toMatch(
      /64/,
    );
  });
});
