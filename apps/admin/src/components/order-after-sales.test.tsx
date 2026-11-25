import type { AdminClient } from "@fastcart/admin-sdk";
import type {
  OrderClaim,
  OrderEdit,
  OrderExchange,
  OrderPaymentSummary,
  OrderReturn,
} from "@fastcart/shared-types";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { OrderAfterSalesPanel } from "@/components/order-after-sales";
import type { OrderAfterSalesData } from "@/lib/order-after-sales";
import type { OrderView } from "@/lib/models";

const order: OrderView = {
  id: "order_1",
  number: 1042,
  customer: "Ava Whitfield",
  email: "ava@example.com",
  placedAt: "2026-01-02T10:00:00Z",
  status: "processing",
  paymentStatus: "captured",
  fulfillmentStatus: "not_fulfilled",
  total: 100,
  shipping: 0,
  tax: 0,
  refundedTotal: 0,
  currency: "USD",
  address: "88 Greenpoint Ave",
  note: null,
  paymentMethod: null,
  shippingMethod: null,
  items: [
    {
      id: "item_1",
      title: "Linen throw",
      variant: "Oat",
      sku: "LLT-OAT",
      quantity: 2,
      price: 50,
    },
  ],
  history: [],
  canFulfill: true,
  canRefund: true,
};

function summary(overrides: Partial<OrderPaymentSummary> = {}): OrderPaymentSummary {
  return {
    order_id: "order_1",
    currency_code: "USD",
    total: 150_00,
    captured_total: 100_00,
    refunded_total: 0,
    net_paid: 100_00,
    outstanding: 50_00,
    refund_due: 0,
    ...overrides,
  };
}

function draftEdit(overrides: Partial<OrderEdit> = {}): OrderEdit {
  return {
    id: "edit_000001",
    order_id: "order_1",
    status: "draft",
    reason: "Add a gift card",
    metadata: {},
    before_subtotal: 100_00,
    after_subtotal: 150_00,
    before_total: 100_00,
    after_total: 150_00,
    difference_total: null,
    created_at: 1,
    updated_at: 1,
    confirmed_at: null,
    canceled_at: null,
    items: [],
    ...overrides,
  };
}

function acceptedClaim(overrides: Partial<OrderClaim> = {}): OrderClaim {
  return {
    id: "claim_000001",
    order_id: "order_1",
    status: "accepted",
    reason: null,
    refund_id: null,
    difference_total: -20_00,
    metadata: {},
    created_at: 1,
    updated_at: 1,
    accepted_at: 2,
    canceled_at: null,
    items: [],
    ...overrides,
  };
}

function acceptedExchange(overrides: Partial<OrderExchange> = {}): OrderExchange {
  return {
    id: "exchange_0001",
    order_id: "order_1",
    status: "accepted",
    reason: null,
    difference_total: 30_00,
    metadata: {},
    created_at: 1,
    updated_at: 1,
    accepted_at: 2,
    canceled_at: null,
    inbound_items: [],
    outbound_items: [],
    ...overrides,
  };
}

function requestedReturn(overrides: Partial<OrderReturn> = {}): OrderReturn {
  return {
    id: "return_000001",
    order_id: "order_1",
    status: "requested",
    reason: "Too small",
    refund_id: null,
    metadata: {},
    created_at: 1,
    updated_at: 1,
    requested_at: 2,
    received_at: null,
    written_off_at: null,
    canceled_at: null,
    items: [],
    ...overrides,
  };
}

function data(overrides: Partial<OrderAfterSalesData> = {}): OrderAfterSalesData {
  return {
    paymentSummary: summary(),
    returns: [],
    claims: [],
    exchanges: [],
    edits: [],
    ...overrides,
  };
}

function fakeClient(): AdminClient {
  return {
    orders: {
      edits: { confirm: vi.fn(async () => draftEdit({ status: "confirmed" })) },
      returns: { receive: vi.fn(async () => requestedReturn()) },
    },
  } as unknown as AdminClient;
}

function renderPanel(
  props: Partial<Parameters<typeof OrderAfterSalesPanel>[0]> = {},
  client: AdminClient = fakeClient(),
) {
  const onChanged = vi.fn();
  render(
    <OrderAfterSalesPanel
      order={order}
      client={client}
      data={data()}
      error={null}
      onRetry={vi.fn()}
      onChanged={onChanged}
      canWrite
      {...props}
    />,
  );
  return { onChanged, client };
}

describe("OrderAfterSalesPanel", () => {
  it("shows the payment summary and flags money the customer still owes", () => {
    renderPanel();

    expect(screen.getByText("Payment summary")).toBeInTheDocument();
    expect(screen.getByText("The customer still owes $50.00.")).toBeInTheDocument();
    expect(screen.getByText("$100.00 net paid")).toBeInTheDocument();
  });

  it("shows a refund due when the order was overpaid", () => {
    renderPanel({
      data: data({
        paymentSummary: summary({
          total: 50_00,
          captured_total: 100_00,
          outstanding: -50_00,
          refund_due: 50_00,
        }),
      }),
    });

    expect(screen.getByText("A refund of $50.00 is due to the customer.")).toBeInTheDocument();
  });

  it("reports when no post-purchase records exist", () => {
    renderPanel();

    expect(
      screen.getByText("No edits, returns, claims or exchanges recorded for this order."),
    ).toBeInTheDocument();
  });

  it("confirms a draft edit and asks the drawer to reload", async () => {
    const { onChanged, client } = renderPanel({
      data: data({ edits: [draftEdit()] }),
    });

    await userEvent.click(screen.getByRole("button", { name: "Confirm" }));

    await waitFor(() =>
      expect(client.orders.edits.confirm).toHaveBeenCalledWith("order_1", "edit_000001"),
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalledTimes(1));
  });

  it("offers a refund for negative differences and a collection for positive ones", () => {
    renderPanel({
      data: data({ claims: [acceptedClaim()], exchanges: [acceptedExchange()] }),
    });

    expect(screen.getByRole("button", { name: "Refund $20.00" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Collect $30.00" })).toBeInTheDocument();
  });

  it("receives every outstanding return unit", async () => {
    const { onChanged, client } = renderPanel({
      data: data({ returns: [requestedReturn()] }),
    });

    await userEvent.click(screen.getByRole("button", { name: "Receive all" }));

    await waitFor(() =>
      expect(client.orders.returns.receive).toHaveBeenCalledWith("order_1", "return_000001", {}),
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it("surfaces a load error with a retry action", async () => {
    const onRetry = vi.fn();
    renderPanel({ data: null, error: "Store is unreachable", onRetry });

    expect(screen.getByText("Store is unreachable")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("offers only order edits while the order is still a draft", () => {
    renderPanel({ order: { ...order, status: "draft" } });

    expect(screen.getByRole("button", { name: "Edit" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Return" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Claim" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Exchange" })).not.toBeInTheDocument();
  });

  it("disables record creation without the orders.write permission", () => {
    renderPanel({ canWrite: false });

    expect(screen.getByRole("button", { name: "Edit" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Return" })).toBeDisabled();
  });
});
