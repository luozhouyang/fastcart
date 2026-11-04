import type { AdminClient } from "@fastcart/admin-sdk";
import { describe, expect, it, vi } from "vitest";

import {
  fulfillOrder,
  loadCustomers,
  loadInventory,
  loadProducts,
  loadSettings,
  orderToView,
  refundOrder,
} from "@/lib/loaders";
import {
  customer,
  domain,
  inventoryItem,
  inventoryLevel,
  location,
  order,
  page,
  payment,
  price,
  product,
  refund,
  store,
} from "@/test/fixtures";

function fakeClient(overrides: Record<string, unknown>): AdminClient {
  return overrides as unknown as AdminClient;
}

describe("orderToView", () => {
  it("maps totals, items, address and history from the API shape", () => {
    const view = orderToView(order(), new Map([["customer-1", "Ava Whitfield"]]), new Map());

    expect(view.customer).toBe("Ava Whitfield");
    expect(view.number).toBe(1042);
    expect(view.total).toBe(220);
    expect(view.items[0]?.price).toBe(184);
    expect(view.address).toContain("88 Greenpoint Ave");
    expect(view.address).toContain("Brooklyn");
    expect(view.note).toBe("Leave with the concierge.");
    expect(view.canFulfill).toBe(true);
    expect(view.canRefund).toBe(true);
    expect(view.history.map((entry) => entry.label)).toEqual([
      "Order placed",
      "Payment captured",
      "Awaiting fulfillment",
    ]);
  });

  it("stops offering refunds once the full total is refunded", () => {
    const view = orderToView(
      order({
        payment_status: "partially_refunded",
        refunds: [refund({ amount: 22_000 })],
      }),
      new Map(),
      new Map([["ava.whitfield@hey.com", "Ava Whitfield"]]),
    );

    expect(view.customer).toBe("Ava Whitfield");
    expect(view.refundedTotal).toBe(220);
    expect(view.canRefund).toBe(false);
  });
});

describe("loadProducts", () => {
  it("joins prices and inventory levels into product views", async () => {
    const client = fakeClient({
      products: { list: vi.fn(async () => page([product()])) },
      priceLists: { prices: vi.fn(async () => page([price()])) },
      inventory: {
        locations: { list: vi.fn(async () => page([location()])) },
        items: { list: vi.fn(async () => page([inventoryItem()])) },
        levels: vi.fn(async () => [inventoryLevel()]),
      },
    });

    const result = await loadProducts(client, store(), new AbortController().signal);
    expect(result.products).toHaveLength(1);
    expect(result.products[0]).toMatchObject({
      title: "Ridge Ceramic Carafe",
      collection: "Tableware",
      price: 58,
      stock: 42,
      status: "active",
    });
    expect(result.counts).toEqual({ all: 1, active: 1, draft: 0, archived: 0 });
    expect(result.collections[0]).toMatchObject({ title: "Tableware", count: 1 });
  });
});

describe("loadInventory", () => {
  it("derives available stock and location totals", async () => {
    const client = fakeClient({
      products: { list: vi.fn(async () => page([product()])) },
      inventory: {
        locations: { list: vi.fn(async () => page([location()])) },
        items: { list: vi.fn(async () => page([inventoryItem()])) },
        levels: vi.fn(async () => [inventoryLevel()]),
      },
    });

    const result = await loadInventory(client, new AbortController().signal);
    expect(result.rows).toHaveLength(1);
    expect(result.rows[0]).toMatchObject({
      product: "Ridge Ceramic Carafe",
      sku: "RCC-SAG-01",
      stocked: 42,
      reserved: 6,
      location: "Brooklyn Studio",
    });
    expect(result.locations[0]).toMatchObject({ units: 42, items: 1 });
    expect(result.totalUnits).toBe(42);
    expect(result.lowCount).toBe(0);
  });
});

describe("loadCustomers", () => {
  it("derives status, spend and metrics from customers and orders", async () => {
    const twoDaysAgo = Math.floor(Date.now() / 1000) - 2 * 86_400;
    const ava = customer();
    const leo = customer({
      id: "customer-2",
      email: "leo.park@outlook.com",
      first_name: "Leo",
      last_name: "Park",
      created_at: twoDaysAgo,
    });

    const client = fakeClient({
      customers: {
        list: vi.fn(async (query?: { limit?: number }) =>
          query?.limit === 1 ? page([], 2) : page([ava, leo], 2),
        ),
      },
      orders: {
        list: vi.fn(async () =>
          page(
            [
              order(),
              order({
                id: "order-2",
                customer_id: "customer-2",
                email: "leo.park@outlook.com",
                total: 13_800,
              }),
            ],
            2,
          ),
        ),
      },
    });

    const result = await loadCustomers(client, new AbortController().signal);
    expect(result.total).toBe(2);
    expect(result.customers).toHaveLength(2);
    expect(result.customers[0]).toMatchObject({
      name: "Ava Whitfield",
      status: "active",
      orders: 1,
      spent: 220,
    });
    expect(result.customers[1]).toMatchObject({
      name: "Leo Park",
      status: "new",
      spent: 138,
    });
    expect(result.metrics.find((metric) => metric.key === "total")?.value).toBe(2);
    expect(result.metrics.find((metric) => metric.key === "returning")?.value).toBe(0);
  });
});

describe("order mutations", () => {
  it("sends refunds in minor units and uses the original payment", async () => {
    const create = vi.fn(async () => order());
    const client = fakeClient({
      payments: { list: vi.fn(async () => page([payment()])) },
      orders: { refunds: { create } },
    });
    const view = orderToView(order(), new Map(), new Map());

    await refundOrder(client, view, 50, "Damaged");
    expect(create).toHaveBeenCalledWith("order-1", {
      payment_id: "payment-1",
      amount: 5000,
      currency: "USD",
      reason: "Damaged",
    });
  });

  it("omits an empty reason and normalizes tracking numbers", async () => {
    const createRefund = vi.fn(async () => order());
    const fulfill = vi.fn(async () => order());
    const client = fakeClient({
      payments: { list: vi.fn(async () => page([payment()])) },
      orders: { refunds: { create: createRefund }, fulfill },
    });
    const view = orderToView(order(), new Map(), new Map());

    await refundOrder(client, view, 25, "   ");
    expect(createRefund).toHaveBeenCalledWith("order-1", {
      payment_id: "payment-1",
      amount: 2500,
      currency: "USD",
    });

    await fulfillOrder(client, "order-1", "  1Z999  ");
    expect(fulfill).toHaveBeenCalledWith("order-1", { tracking_numbers: ["1Z999"] });

    await fulfillOrder(client, "order-1", "");
    expect(fulfill).toHaveBeenLastCalledWith("order-1", { tracking_numbers: [] });
  });
});

describe("loadSettings", () => {
  it("combines store settings with the platform domain list", async () => {
    const client = fakeClient({
      settings: {
        get: vi.fn(async () => ({
          support_email: "help@halcyon.test",
          weight_unit: "lb",
          guest_checkout: "false",
          require_phone: "true",
        })),
      },
      stores: { domains: { list: vi.fn(async () => [domain()]) } },
    });

    const result = await loadSettings(client, store(), new AbortController().signal);
    expect(result).toMatchObject({
      name: "Halcyon Supply",
      handle: "halcyon-supply",
      supportEmail: "help@halcyon.test",
      weightUnit: "lb",
      primaryDomain: "halcyonsupply.test",
      checkout: {
        guestCheckout: false,
        requirePhone: true,
        confirmationEmails: true,
      },
    });
  });
});
