import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { API_BASE_URL } from "@/lib/api";
import {
  campaignBudgetFraction,
  campaignBudgetLabel,
  campaignBudgetProblem,
  createCampaign,
  deleteCampaign,
  linkablePromotions,
  linkCampaignPromotion,
  listCampaigns,
  listCampaignPromotions,
  unlinkCampaignPromotion,
  updateCampaign,
  type Campaign,
  type CreateCampaignInput,
} from "@/lib/campaigns";

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

function campaign(overrides: Partial<Campaign> = {}): Campaign {
  return {
    id: "camp_1",
    name: "Autumn launch",
    description: null,
    budget_type: "usage",
    currency_code: null,
    budget_limit: 500,
    budget_used: 42,
    starts_at: null,
    ends_at: null,
    metadata: {},
    created_at: 1,
    updated_at: 2,
    ...overrides,
  };
}

function createInput(overrides: Partial<CreateCampaignInput> = {}): CreateCampaignInput {
  return {
    name: "Autumn launch",
    description: null,
    budget_type: "usage",
    currency_code: null,
    budget_limit: 500,
    starts_at: null,
    ends_at: null,
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

describe("campaign client", () => {
  it("lists campaigns with store, csrf and pagination", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [campaign()], pagination: { limit: 10, offset: 0, total: 1 } }),
    );
    vi.stubGlobal("fetch", mock);

    const result = await listCampaigns("store-1", { limit: 10, offset: 0 });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/campaigns?limit=10&offset=0`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
    expect(result.data[0]?.name).toBe("Autumn launch");
  });

  it("forwards an abort signal", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [], pagination: { limit: 0, offset: 0, total: 0 } }),
    );
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listCampaigns("store-1", {}, controller.signal);

    expect(requestAt(mock).init.signal).toBe(controller.signal);
  });

  it("creates a campaign with a JSON body", async () => {
    const mock = vi.fn(async () => jsonResponse(campaign(), 201));
    vi.stubGlobal("fetch", mock);

    await createCampaign(
      "store-1",
      createInput({ budget_type: "spend", currency_code: "USD", budget_limit: 50_000 }),
    );

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/campaigns`);
    expect(init.method).toBe("POST");
    expect((init.headers as Headers).get("content-type")).toBe("application/json");
    expect(JSON.parse(String(init.body))).toMatchObject({
      name: "Autumn launch",
      budget_type: "spend",
      currency_code: "USD",
      budget_limit: 50_000,
    });
  });

  it("patches a campaign, clearing nullable fields with null", async () => {
    const mock = vi.fn(async () => jsonResponse(campaign({ budget_limit: null })));
    vi.stubGlobal("fetch", mock);

    await updateCampaign("store-1", "camp 1", { budget_limit: null, ends_at: null });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/campaigns/camp%201`);
    expect(init.method).toBe("PATCH");
    expect(JSON.parse(String(init.body))).toEqual({ budget_limit: null, ends_at: null });
  });

  it("deletes a campaign and accepts an empty 204", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(deleteCampaign("store-1", "camp_1")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/campaigns/camp_1`);
    expect(init.method).toBe("DELETE");
  });

  it("lists linked promotions as ids", async () => {
    const mock = vi.fn(async () => jsonResponse(["promo_1"]));
    vi.stubGlobal("fetch", mock);

    await expect(listCampaignPromotions("store-1", "camp_1")).resolves.toEqual(["promo_1"]);

    expect(requestAt(mock).url).toBe(`${API_BASE_URL}/api/v1/admin/campaigns/camp_1/promotions`);
  });

  it("links and unlinks a promotion", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await linkCampaignPromotion("store-1", "camp_1", "promo_1");
    await unlinkCampaignPromotion("store-1", "camp_1", "promo_1");

    const link = requestAt(mock, 0);
    expect(link.url).toBe(`${API_BASE_URL}/api/v1/admin/campaigns/camp_1/promotions`);
    expect(link.init.method).toBe("POST");
    expect(JSON.parse(String(link.init.body))).toEqual({ promotion_id: "promo_1" });

    const unlink = requestAt(mock, 1);
    expect(unlink.url).toBe(`${API_BASE_URL}/api/v1/admin/campaigns/camp_1/promotions/promo_1`);
    expect(unlink.init.method).toBe("DELETE");
  });

  it("surfaces the API error envelope", async () => {
    const mock = vi.fn(async () =>
      jsonResponse(
        { error: { code: "VALIDATION_ERROR", message: "campaign budget must be positive" } },
        422,
      ),
    );
    vi.stubGlobal("fetch", mock);

    await expect(createCampaign("store-1", createInput({ budget_limit: 0 }))).rejects.toThrow(
      "campaign budget must be positive",
    );
  });

  it("falls back to the http status without an error body", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 500));
    vi.stubGlobal("fetch", mock);

    await expect(deleteCampaign("store-1", "camp_1")).rejects.toThrow(
      "Campaign request failed (500)",
    );
  });
});

describe("campaign helpers", () => {
  it("requires a currency and limit for spend budgets", () => {
    expect(campaignBudgetProblem("spend", null, 1_000)).toBe("A spend budget requires a currency.");
    expect(campaignBudgetProblem("spend", "USD", null)).toBe("A spend budget requires a limit.");
    expect(campaignBudgetProblem("spend", "USD", 1_000)).toBeNull();
    expect(campaignBudgetProblem("usage", null, 1_000)).toBeNull();
    expect(campaignBudgetProblem("usage", null, 0)).toBe("Budget must be a positive whole number.");
  });

  it("formats the consumed budget for each budget type", () => {
    expect(campaignBudgetLabel(campaign({ budget_type: "usage", budget_limit: 500 }))).toBe(
      "42 of 500",
    );
    expect(campaignBudgetLabel(campaign({ budget_type: "usage", budget_limit: null }))).toBe(
      "42 uses",
    );
    expect(
      campaignBudgetLabel(
        campaign({
          budget_type: "spend",
          currency_code: "USD",
          budget_used: 7_850,
          budget_limit: 25_000,
        }),
      ),
    ).toBe("$78.50 of $250.00");
    expect(
      campaignBudgetLabel(
        campaign({
          budget_type: "spend",
          currency_code: null,
          budget_used: 900,
          budget_limit: null,
        }),
      ),
    ).toBe("$9.00 spent");
  });

  it("computes the consumed budget share", () => {
    expect(campaignBudgetFraction(25, 100)).toBe(0.25);
    expect(campaignBudgetFraction(150, 100)).toBe(1);
    expect(campaignBudgetFraction(10, null)).toBeNull();
    expect(campaignBudgetFraction(10, 0)).toBeNull();
  });

  it("offers only unlinked promotions, sorted by title", () => {
    const promotions = [
      { id: "p2", title: "Winter" },
      { id: "p1", title: "Autumn" },
      { id: "p3", title: "Linked" },
    ];
    expect(linkablePromotions(promotions, ["p3"]).map((entry) => entry.id)).toEqual(["p1", "p2"]);
  });
});
