import { describe, expect, it } from "vitest";

import {
  demoInventory,
  demoOrdersPage,
  demoOverview,
  demoProducts,
  demoSettings,
  orderView,
} from "@/lib/demo";
import { orders as mockOrders } from "@/data/mock";

describe("demo order views", () => {
  it("maps a mock order into the shared view model", () => {
    const source = mockOrders[0];
    expect(source).toBeDefined();
    if (!source) return;

    const view = orderView(source);
    expect(view.number).toBe(1042);
    expect(view.currency).toBe("USD");
    expect(view.total).toBe(220);
    expect(view.items).toHaveLength(2);
    expect(view.canFulfill).toBe(true);
    expect(view.canRefund).toBe(true);
    expect(view.history.map((entry) => entry.label)).toContain("Payment captured");
  });

  it("marks a refunded order as no longer refundable", () => {
    const source = mockOrders.find((order) => order.paymentStatus === "refunded");
    expect(source).toBeDefined();
    if (!source) return;
    const view = orderView(source);
    expect(view.refundedTotal).toBe(view.total);
    expect(view.canRefund).toBe(false);
    expect(view.canFulfill).toBe(false);
  });
});

describe("demo order paging", () => {
  it("keeps the designed total while paging the sample", () => {
    const first = demoOrdersPage({
      status: "all",
      page: 1,
      pageSize: 10,
      query: "",
      range: "all",
      sort: "newest",
    });
    expect(first.orders).toHaveLength(10);
    expect(first.total).toBe(1284);
    expect(first.loaded).toBe(11);
    expect(first.orders[0]?.number).toBe(1042);

    const second = demoOrdersPage({
      status: "all",
      page: 2,
      pageSize: 10,
      query: "",
      range: "all",
      sort: "newest",
    });
    expect(second.orders).toHaveLength(1);
    expect(second.orders[0]?.number).toBe(1032);
  });

  it("filters by status and search", () => {
    const pending = demoOrdersPage({
      status: "pending",
      page: 1,
      pageSize: 10,
      query: "",
      range: "all",
      sort: "newest",
    });
    expect(pending.orders.every((order) => order.status === "pending")).toBe(true);
    expect(pending.loaded).toBe(1);
    expect(pending.total).toBe(164);

    const search = demoOrdersPage({
      status: "all",
      page: 1,
      pageSize: 10,
      query: "ava",
      range: "all",
      sort: "newest",
    });
    expect(search.orders).toHaveLength(1);
    expect(search.total).toBe(1);
    expect(search.orders[0]?.customer).toBe("Ava Whitfield");
  });

  it("sorts by total descending", () => {
    const sorted = demoOrdersPage({
      status: "all",
      page: 1,
      pageSize: 10,
      query: "",
      range: "all",
      sort: "total",
    });
    const totals = sorted.orders.map((order) => order.total);
    expect(totals).toEqual([...totals].sort((a, b) => b - a));
    expect(sorted.orders[0]?.total).toBe(256);
  });
});

describe("demo overview and catalog", () => {
  it("provides every revenue range and the designed metrics", () => {
    const overview = demoOverview();
    expect(Object.keys(overview.revenueRanges)).toEqual(["7d", "30d", "90d"]);
    expect(overview.revenueRanges["30d"]).toHaveLength(30);
    expect(overview.metrics.find((metric) => metric.key === "revenue")?.display).toBe(true);
    expect(overview.statusSlices.length).toBeGreaterThan(0);
    expect(overview.recentOrders).toHaveLength(5);
  });

  it("derives product counts from the sample", () => {
    const catalog = demoProducts();
    expect(catalog.counts).toEqual({ all: 8, active: 7, draft: 1, archived: 0 });
    expect(catalog.collections).toHaveLength(4);
    expect(catalog.products.find((product) => product.title === "Tide Glass Vessel")?.status).toBe(
      "draft",
    );
  });

  it("derives inventory totals from the rows", () => {
    const inventory = demoInventory();
    const stocked = inventory.rows.reduce((total, row) => total + row.stocked, 0);
    expect(inventory.totalUnits).toBe(stocked);
    expect(inventory.locations).toHaveLength(3);
    expect(inventory.lowCount).toBe(2);
  });

  it("ships the settings shown in the review", () => {
    const settings = demoSettings();
    expect(settings.primaryDomain).toBe("halcyonsupply.com");
    expect(settings.domains).toHaveLength(2);
    expect(settings.checkout.guestCheckout).toBe(true);
  });
});
