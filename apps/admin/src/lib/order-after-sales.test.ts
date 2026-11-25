import type { AdminClient } from "@fastcart/admin-sdk";
import { describe, expect, it, vi } from "vitest";

import {
  buildClaimRequest,
  buildExchangeRequest,
  buildOrderEditRequest,
  buildReturnRequest,
  loadOrderAfterSales,
} from "@/lib/order-after-sales";
import type { OrderItemView } from "@/lib/models";

const items: OrderItemView[] = [
  {
    id: "item_1",
    title: "Linen throw",
    variant: "Oat",
    sku: "LLT-OAT",
    quantity: 2,
    price: 96,
  },
  {
    id: "item_2",
    title: "Stone soap",
    variant: "Cedar",
    sku: "BSS-CDR",
    quantity: 3,
    price: 18,
  },
];

describe("buildOrderEditRequest", () => {
  it("omits unchanged lines and turns a zero target into a remove", () => {
    const request = buildOrderEditRequest(
      items,
      { item_1: 1, item_2: 0 },
      "  customer trimmed the order  ",
    );

    expect(request.reason).toBe("customer trimmed the order");
    expect(request.metadata).toEqual({});
    expect(request.items).toEqual([
      {
        order_item_id: "item_1",
        action: "update",
        before_quantity: 2,
        after_quantity: 1,
      },
      {
        order_item_id: "item_2",
        action: "remove",
        before_quantity: 3,
        after_quantity: 0,
      },
    ]);
  });

  it("refuses an edit that changes nothing", () => {
    expect(() => buildOrderEditRequest(items, { item_1: 2, item_2: 3 }, "")).toThrow(
      "Change at least one quantity.",
    );
  });

  it("rejects negative and fractional targets", () => {
    expect(() => buildOrderEditRequest(items, { item_1: -1 }, "")).toThrow(
      "Enter a whole quantity for Linen throw.",
    );
    expect(() => buildOrderEditRequest(items, { item_1: 1.5 }, "")).toThrow(
      "Enter a whole quantity for Linen throw.",
    );
  });

  it("appends an explicitly priced new line", () => {
    const request = buildOrderEditRequest(items, {}, "", {
      productTitle: "  Gift card  ",
      unitPrice: 2_500,
      quantity: 2,
    });

    expect(request.items).toEqual([
      {
        action: "add",
        product_title: "Gift card",
        unit_price: 2_500,
        after_quantity: 2,
        requires_shipping: true,
      },
    ]);
  });

  it("validates the new line", () => {
    expect(() =>
      buildOrderEditRequest(items, {}, "", { productTitle: " ", unitPrice: 0, quantity: 1 }),
    ).toThrow("Enter a title for the line to add.");
    expect(() =>
      buildOrderEditRequest(items, {}, "", { productTitle: "Gift", unitPrice: 0, quantity: 0 }),
    ).toThrow("Enter a whole quantity greater than zero for the new line.");
    expect(() =>
      buildOrderEditRequest(items, {}, "", { productTitle: "Gift", unitPrice: -1, quantity: 1 }),
    ).toThrow("Enter a unit price of zero or more for the new line.");
  });
});

describe("buildReturnRequest", () => {
  it("includes only lines with a positive quantity", () => {
    const request = buildReturnRequest(items, { item_1: 0, item_2: 2 }, "");

    expect(request.reason).toBeNull();
    expect(request.items).toEqual([{ order_item_id: "item_2", quantity: 2 }]);
  });

  it("refuses an empty return", () => {
    expect(() => buildReturnRequest(items, {}, "")).toThrow("Choose at least one item to return.");
  });

  it("refuses returning more than the purchased quantity", () => {
    expect(() => buildReturnRequest(items, { item_1: 3 }, "")).toThrow(
      "Return between 0 and 2 of Linen throw.",
    );
  });
});

describe("buildClaimRequest", () => {
  it("builds refund-type claim lines", () => {
    const request = buildClaimRequest(items, { item_1: 1 }, "arrived damaged");

    expect(request.reason).toBe("arrived damaged");
    expect(request.items).toEqual([{ order_item_id: "item_1", quantity: 1, item_type: "refund" }]);
  });

  it("refuses an empty claim", () => {
    expect(() => buildClaimRequest(items, { item_1: 0 }, "")).toThrow(
      "Choose at least one item to claim.",
    );
  });
});

describe("buildExchangeRequest", () => {
  it("mirrors inbound lines onto the replacement variant", () => {
    const request = buildExchangeRequest(items, { item_1: 1 }, "variant_sage", "");

    expect(request.inbound_items).toEqual([{ order_item_id: "item_1", quantity: 1 }]);
    expect(request.outbound_items).toEqual([
      {
        order_item_id: "item_1",
        quantity: 1,
        replacement_variant_id: "variant_sage",
      },
    ]);
  });

  it("requires a replacement variant", () => {
    expect(() => buildExchangeRequest(items, { item_1: 1 }, "  ", "")).toThrow(
      "Enter the replacement variant id.",
    );
  });

  it("refuses an exchange without inbound units", () => {
    expect(() => buildExchangeRequest(items, {}, "variant_sage", "")).toThrow(
      "Choose at least one item to exchange.",
    );
  });
});

describe("loadOrderAfterSales", () => {
  it("loads the payment summary and every after-sales list together", async () => {
    const signal = new AbortController().signal;
    const paymentSummary = vi.fn(async () => ({ outstanding: 0 }));
    const list = vi.fn(async () => []);
    const client = {
      orders: {
        paymentSummary,
        returns: { list },
        claims: { list },
        exchanges: { list },
        edits: { list },
      },
    } as unknown as AdminClient;

    const data = await loadOrderAfterSales(client, "order_1", signal);

    expect(paymentSummary).toHaveBeenCalledWith("order_1", { signal });
    expect(list).toHaveBeenCalledTimes(4);
    expect(data.paymentSummary.outstanding).toBe(0);
    expect(data.returns).toEqual([]);
    expect(data.claims).toEqual([]);
    expect(data.exchanges).toEqual([]);
    expect(data.edits).toEqual([]);
  });
});
