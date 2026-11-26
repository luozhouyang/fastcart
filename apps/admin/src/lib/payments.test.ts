import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { API_BASE_URL } from "@/lib/api";
import {
  canRefundPayment,
  cancelPaymentSession,
  capturePaymentSession,
  createPaymentRefund,
  failPaymentSession,
  getPayment,
  listPaymentRefunds,
  listPayments,
  paymentFilterOptions,
  paymentMethodLabel,
  paymentSessionId,
  paymentStatusLabel,
  paymentStatusTone,
  refundableMinor,
  refundedMinor,
  type AdminPayment,
} from "@/lib/payments";
import type { PaymentRefund } from "@fastcart/shared-types";

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

function payment(overrides: Partial<AdminPayment> = {}): AdminPayment {
  return {
    id: "pay_1",
    order_id: "order-1",
    provider: "manual",
    provider_payment_id: "manual_pay_1",
    amount: 12_000,
    currency_code: "USD",
    status: "captured",
    captured_at: 10,
    metadata: {},
    created_at: 10,
    updated_at: 11,
    ...overrides,
  };
}

function refund(overrides: Partial<PaymentRefund> = {}): PaymentRefund {
  return {
    id: "refund-1",
    payment_id: "pay_1",
    order_id: "order-1",
    amount: 2_000,
    currency_code: "USD",
    status: "succeeded",
    provider_refund_id: null,
    reason: "Damaged item",
    created_at: 12,
    updated_at: 12,
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

describe("payment client", () => {
  it("lists payments with store, csrf and filters", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ data: [payment()], pagination: { limit: 20, offset: 0, total: 1 } }),
    );
    vi.stubGlobal("fetch", mock);

    const result = await listPayments("store-1", {
      order_id: "order 1",
      status: "captured",
      limit: 20,
      offset: 0,
    });

    const { url, init } = requestAt(mock);
    expect(url).toBe(
      `${API_BASE_URL}/api/v1/admin/payments?order_id=order+1&status=captured&limit=20&offset=0`,
    );
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
    expect(result.data[0]?.id).toBe("pay_1");
  });

  it("forwards an abort signal", async () => {
    const mock = vi.fn(async () => jsonResponse(payment()));
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await getPayment("store-1", "pay_1", controller.signal);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/payments/pay_1`);
    expect(init.signal).toBe(controller.signal);
  });

  it("lists refunds for a payment", async () => {
    const mock = vi.fn(async () => jsonResponse([refund()]));
    vi.stubGlobal("fetch", mock);

    await expect(listPaymentRefunds("store-1", "pay_1")).resolves.toEqual([refund()]);

    expect(requestAt(mock).url).toBe(`${API_BASE_URL}/api/v1/admin/payments/pay_1/refunds`);
  });

  it("creates a refund with a JSON body", async () => {
    const mock = vi.fn(async () => jsonResponse(refund(), 201));
    vi.stubGlobal("fetch", mock);

    await createPaymentRefund("store-1", "pay_1", { amount: 2_000, reason: "Damaged item" });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/payments/pay_1/refunds`);
    expect(init.method).toBe("POST");
    expect((init.headers as Headers).get("content-type")).toBe("application/json");
    expect(JSON.parse(String(init.body))).toEqual({ amount: 2_000, reason: "Damaged item" });
  });

  it("captures a payment session", async () => {
    const mock = vi.fn(async () => jsonResponse(payment()));
    vi.stubGlobal("fetch", mock);

    const captured = await capturePaymentSession("store-1", "sess 1");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/payment-sessions/sess%201/capture`);
    expect(init.method).toBe("POST");
    expect(captured.status).toBe("captured");
  });

  it("cancels a payment session", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ id: "sess_1", status: "canceled", amount: 1_000, currency_code: "USD" }),
    );
    vi.stubGlobal("fetch", mock);

    await cancelPaymentSession("store-1", "sess_1");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/payment-sessions/sess_1/cancel`);
    expect(init.method).toBe("POST");
  });

  it("fails a payment session with a reason", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ id: "sess_1", status: "failed", amount: 1_000, currency_code: "USD" }),
    );
    vi.stubGlobal("fetch", mock);

    await failPaymentSession("store-1", "sess_1", { reason: "card declined" });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/payment-sessions/sess_1/fail`);
    expect(init.method).toBe("POST");
    expect(JSON.parse(String(init.body))).toEqual({ reason: "card declined" });
  });

  it("surfaces the API error envelope", async () => {
    const mock = vi.fn(async () =>
      jsonResponse(
        { error: { code: "CONFLICT", message: "cannot refund a payment in status pending" } },
        409,
      ),
    );
    vi.stubGlobal("fetch", mock);

    await expect(createPaymentRefund("store-1", "pay_1", { amount: 500 })).rejects.toThrow(
      "cannot refund a payment in status pending",
    );
  });

  it("falls back to the http status without an error body", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 500));
    vi.stubGlobal("fetch", mock);

    await expect(getPayment("store-1", "pay_1")).rejects.toThrow("Payment request failed (500)");
  });
});

describe("payment helpers", () => {
  it("labels known statuses and passes unknown ones through", () => {
    expect(paymentStatusLabel("partially_captured")).toBe("Partially captured");
    expect(paymentStatusLabel("settled")).toBe("settled");
    expect(paymentStatusTone("refunded")).toBe("destructive");
    expect(paymentStatusTone("settled")).toBe("secondary");
  });

  it("builds the status filter with an all option first", () => {
    const options = paymentFilterOptions();
    expect(options[0]).toEqual({ value: "all", label: "All" });
    expect(options.map((option) => option.value)).toContain("partially_captured");
    expect(options.find((option) => option.value === "requires_action")?.label).toBe(
      "Requires action",
    );
  });

  it("labels the payment method from the provider", () => {
    expect(paymentMethodLabel(payment({ provider: "stripe" }))).toBe("stripe");
    expect(paymentMethodLabel(payment({ provider: "  " }))).toBe("Manual");
  });

  it("reads a session id from payment metadata when present", () => {
    expect(paymentSessionId(payment({ metadata: { session_id: "sess_1" } }))).toBe("sess_1");
    expect(paymentSessionId(payment({ metadata: { payment_session_id: "sess_2" } }))).toBe(
      "sess_2",
    );
    expect(paymentSessionId(payment({ metadata: { session_id: 7 } }))).toBeNull();
    expect(paymentSessionId(payment())).toBeNull();
  });

  it("sums successful refunds only", () => {
    expect(
      refundedMinor([refund(), refund({ id: "refund-2", amount: 500, status: "failed" })]),
    ).toBe(2_000);
  });

  it("computes the refundable remainder and availability", () => {
    const captured = payment({ amount: 5_000 });
    expect(refundableMinor(captured, [refund({ amount: 1_500 })])).toBe(3_500);
    expect(canRefundPayment(captured, [])).toBe(true);
    expect(canRefundPayment(payment({ status: "refunded" }), [])).toBe(false);
    expect(canRefundPayment(payment({ status: "pending" }), [])).toBe(false);
    expect(canRefundPayment(payment({ amount: 2_000 }), [refund({ amount: 2_000 })])).toBe(false);
    expect(canRefundPayment(payment({ status: "partially_refunded" }), [])).toBe(true);
  });
});
