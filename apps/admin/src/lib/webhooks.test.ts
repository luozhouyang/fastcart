import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { API_BASE_URL } from "@/lib/api";
import {
  canReplayDelivery,
  createWebhook,
  deleteWebhook,
  eventsLabel,
  listWebhookDeliveries,
  listWebhooks,
  parseEventList,
  replayWebhookDelivery,
  rotateWebhookSecret,
  updateWebhook,
  webhookUrlError,
  type WebhookDelivery,
  type WebhookEndpoint,
} from "@/lib/webhooks";

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

function endpoint(overrides: Partial<WebhookEndpoint> = {}): WebhookEndpoint {
  return {
    id: "wh_1",
    url: "https://example.com/hooks",
    events: ["order.placed"],
    is_active: true,
    metadata: {},
    created_at: 1,
    updated_at: 1,
    ...overrides,
  };
}

function delivery(overrides: Partial<WebhookDelivery> = {}): WebhookDelivery {
  return {
    id: "whd_1",
    endpoint_id: "wh_1",
    event_id: "evt_1",
    event_type: "order.placed",
    payload: {},
    status: "failed",
    attempts: 3,
    response_status: 500,
    response_body: "boom",
    next_retry_at: null,
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

describe("webhook client", () => {
  it("lists endpoints with store and csrf headers", async () => {
    const mock = vi.fn(async () => jsonResponse([endpoint()]));
    vi.stubGlobal("fetch", mock);

    await expect(listWebhooks("store-1")).resolves.toEqual([endpoint()]);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/webhooks`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("forwards an abort signal", async () => {
    const mock = vi.fn(async () => jsonResponse([]));
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listWebhooks("store-1", controller.signal);

    expect(requestAt(mock).init.signal).toBe(controller.signal);
  });

  it("creates an endpoint and returns the one-time secret", async () => {
    const mock = vi.fn(async () => jsonResponse({ ...endpoint(), secret: "whsec_created" }, 201));
    vi.stubGlobal("fetch", mock);

    const created = await createWebhook("store-1", {
      url: "https://example.com/hooks",
      events: ["order.placed"],
    });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/webhooks`);
    expect(init.method).toBe("POST");
    expect((init.headers as Headers).get("content-type")).toBe("application/json");
    expect(JSON.parse(String(init.body))).toEqual({
      url: "https://example.com/hooks",
      events: ["order.placed"],
    });
    expect(created.secret).toBe("whsec_created");
  });

  it("patches url, events and active state", async () => {
    const mock = vi.fn(async () => jsonResponse(endpoint({ is_active: false, events: ["*"] })));
    vi.stubGlobal("fetch", mock);

    const updated = await updateWebhook("store-1", "wh 1", { events: ["*"], is_active: false });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/webhooks/wh%201`);
    expect(init.method).toBe("PATCH");
    expect(JSON.parse(String(init.body))).toEqual({ events: ["*"], is_active: false });
    expect(updated.is_active).toBe(false);
  });

  it("deletes an endpoint and accepts an empty 204", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(deleteWebhook("store-1", "wh_1")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/webhooks/wh_1`);
    expect(init.method).toBe("DELETE");
  });

  it("rotates the signing secret", async () => {
    const mock = vi.fn(async () => jsonResponse({ secret: "whsec_rotated" }));
    vi.stubGlobal("fetch", mock);

    await expect(rotateWebhookSecret("store-1", "wh_1")).resolves.toEqual({
      secret: "whsec_rotated",
    });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/webhooks/wh_1/rotate-secret`);
    expect(init.method).toBe("POST");
  });

  it("lists deliveries with a limit", async () => {
    const mock = vi.fn(async () => jsonResponse([delivery()]));
    vi.stubGlobal("fetch", mock);

    await listWebhookDeliveries("store-1", "wh_1", { limit: 25 });

    expect(requestAt(mock).url).toBe(
      `${API_BASE_URL}/api/v1/admin/webhooks/wh_1/deliveries?limit=25`,
    );
  });

  it("replays a delivery", async () => {
    const mock = vi.fn(async () => jsonResponse(delivery({ status: "pending", attempts: 0 })));
    vi.stubGlobal("fetch", mock);

    const replayed = await replayWebhookDelivery("store-1", "whd_1");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/webhook-deliveries/whd_1/replay`);
    expect(init.method).toBe("POST");
    expect(replayed.status).toBe("pending");
  });

  it("surfaces the API error envelope", async () => {
    const mock = vi.fn(async () =>
      jsonResponse(
        { error: { code: "CONFLICT", message: "webhook delivery is currently being processed" } },
        409,
      ),
    );
    vi.stubGlobal("fetch", mock);

    await expect(replayWebhookDelivery("store-1", "whd_1")).rejects.toThrow(
      "webhook delivery is currently being processed",
    );
  });

  it("falls back to the http status without an error body", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 502));
    vi.stubGlobal("fetch", mock);

    await expect(listWebhooks("store-1")).rejects.toThrow("Webhook request failed (502)");
  });
});

describe("webhook helpers", () => {
  it("normalizes a free-form event list", () => {
    expect(parseEventList("order.placed, payment.captured\n order.placed")).toEqual([
      "order.placed",
      "payment.captured",
    ]);
    expect(parseEventList("  ")).toEqual([]);
  });

  it("labels subscriptions", () => {
    expect(eventsLabel([])).toBe("No events");
    expect(eventsLabel(["*"])).toBe("All events");
    expect(eventsLabel(["order.placed"])).toBe("order.placed");
    expect(eventsLabel(["order.placed", "payment.captured"])).toBe("order.placed +1");
  });

  it("blocks replays only while a delivery is in flight", () => {
    expect(canReplayDelivery("delivering")).toBe(false);
    expect(canReplayDelivery("failed")).toBe(true);
    expect(canReplayDelivery("delivered")).toBe(true);
  });

  it("requires an absolute https url", () => {
    expect(webhookUrlError("")).toBe("Enter the URL that should receive events.");
    expect(webhookUrlError("hooks")).toBe(
      "Enter a complete URL, for example https://example.com/hooks.",
    );
    expect(webhookUrlError("http://example.com/hooks")).toBe("Use an https URL.");
    expect(webhookUrlError("http://localhost:3000/hooks", true)).toBeNull();
    expect(webhookUrlError("https://example.com/hooks")).toBeNull();
  });
});
