import type { AdminClient } from "@fastcart/admin-sdk";
import type {
  ClaimItemRequest,
  CreateClaimRequest,
  CreateExchangeRequest,
  CreateOrderEditRequest,
  CreateReturnRequest,
  OrderClaim,
  OrderEdit,
  OrderExchange,
  OrderPaymentSummary,
  OrderReturn,
} from "@fastcart/shared-types";

import type { OrderItemView } from "@/lib/models";

/** Everything the order drawer shows about money and post-purchase work. */
export interface OrderAfterSalesData {
  paymentSummary: OrderPaymentSummary;
  returns: OrderReturn[];
  claims: OrderClaim[];
  exchanges: OrderExchange[];
  edits: OrderEdit[];
}

export async function loadOrderAfterSales(
  client: AdminClient,
  orderId: string,
  signal: AbortSignal,
): Promise<OrderAfterSalesData> {
  const [paymentSummary, returns, claims, exchanges, edits] = await Promise.all([
    client.orders.paymentSummary(orderId, { signal }),
    client.orders.returns.list(orderId, { signal }),
    client.orders.claims.list(orderId, { signal }),
    client.orders.exchanges.list(orderId, { signal }),
    client.orders.edits.list(orderId, { signal }),
  ]);
  return { paymentSummary, returns, claims, exchanges, edits };
}

function orderItemId(item: OrderItemView): string {
  if (!item.id) {
    throw new Error("This order line has no identifier.");
  }
  return item.id;
}

function requestedQuantity(quantities: Record<string, number>, item: OrderItemView): number {
  const value = quantities[item.id ?? item.sku];
  return value === undefined ? 0 : value;
}

/** A new line appended to an order through an edit. */
export interface NewOrderLine {
  productTitle: string;
  /** Unit price in minor units. */
  unitPrice: number;
  quantity: number;
  requiresShipping?: boolean;
}

/**
 * Builds an edit request from per-item target quantities.
 *
 * Unchanged lines are omitted; a target of zero removes the line. A supplied
 * new line is appended as an explicit `add`, which the backend prices from its
 * own unit price. Draft orders are re-taxed when they are converted, so the
 * added line enters the placed order with the destination's tax.
 */
export function buildOrderEditRequest(
  items: OrderItemView[],
  quantities: Record<string, number>,
  reason: string,
  newLine?: NewOrderLine,
): CreateOrderEditRequest {
  const lines: CreateOrderEditRequest["items"] = items.flatMap((item) => {
    const target = quantities[item.id ?? item.sku];
    if (target === undefined || target === item.quantity) return [];
    if (!Number.isInteger(target) || target < 0) {
      throw new Error(`Enter a whole quantity for ${item.title}.`);
    }
    return [
      {
        order_item_id: orderItemId(item),
        action: (target === 0 ? "remove" : "update") as "remove" | "update",
        before_quantity: item.quantity,
        after_quantity: target,
      },
    ];
  });
  if (newLine) {
    const title = newLine.productTitle.trim();
    if (title.length === 0) {
      throw new Error("Enter a title for the line to add.");
    }
    if (!Number.isInteger(newLine.quantity) || newLine.quantity < 1) {
      throw new Error("Enter a whole quantity greater than zero for the new line.");
    }
    if (!Number.isInteger(newLine.unitPrice) || newLine.unitPrice < 0) {
      throw new Error("Enter a unit price of zero or more for the new line.");
    }
    lines.push({
      action: "add",
      product_title: title,
      unit_price: newLine.unitPrice,
      after_quantity: newLine.quantity,
      requires_shipping: newLine.requiresShipping ?? true,
    });
  }
  if (lines.length === 0) {
    throw new Error("Change at least one quantity.");
  }
  return {
    reason: reason.trim().length > 0 ? reason.trim() : null,
    metadata: {},
    items: lines,
  };
}

/** Builds a return request from per-item return quantities. */
export function buildReturnRequest(
  items: OrderItemView[],
  quantities: Record<string, number>,
  reason: string,
  reasonId = "",
): CreateReturnRequest {
  const lines = items.flatMap((item) => {
    const quantity = requestedQuantity(quantities, item);
    if (quantity === 0) return [];
    if (!Number.isInteger(quantity) || quantity < 0 || quantity > item.quantity) {
      throw new Error(`Return between 0 and ${item.quantity} of ${item.title}.`);
    }
    return [{ order_item_id: orderItemId(item), quantity }];
  });
  if (lines.length === 0) {
    throw new Error("Choose at least one item to return.");
  }
  return {
    reason: reason.trim().length > 0 ? reason.trim() : null,
    // A configured reason takes precedence server-side; its label is
    // snapshotted onto the record.
    ...(reasonId.length > 0 ? { reason_id: reasonId } : {}),
    metadata: {},
    items: lines,
  };
}

/** Builds a refund-type claim request from per-item claim quantities. */
export function buildClaimRequest(
  items: OrderItemView[],
  quantities: Record<string, number>,
  reason: string,
  reasonId = "",
): CreateClaimRequest {
  const lines: ClaimItemRequest[] = items.flatMap((item) => {
    const quantity = requestedQuantity(quantities, item);
    if (quantity === 0) return [];
    if (!Number.isInteger(quantity) || quantity < 0 || quantity > item.quantity) {
      throw new Error(`Claim between 0 and ${item.quantity} of ${item.title}.`);
    }
    return [{ order_item_id: orderItemId(item), quantity, item_type: "refund" as const }];
  });
  if (lines.length === 0) {
    throw new Error("Choose at least one item to claim.");
  }
  return {
    reason: reason.trim().length > 0 ? reason.trim() : null,
    ...(reasonId.length > 0 ? { reason_id: reasonId } : {}),
    metadata: {},
    items: lines,
  };
}

/**
 * Builds an exchange request from per-item inbound quantities.
 *
 * Every inbound unit is replaced by the same quantity of one replacement
 * variant, because an exchange line must name the variant that ships back.
 */
export function buildExchangeRequest(
  items: OrderItemView[],
  quantities: Record<string, number>,
  replacementVariantId: string,
  reason: string,
  reasonId = "",
): CreateExchangeRequest {
  const variantId = replacementVariantId.trim();
  if (variantId.length === 0) {
    throw new Error("Enter the replacement variant id.");
  }
  const inbound = items.flatMap((item) => {
    const quantity = requestedQuantity(quantities, item);
    if (quantity === 0) return [];
    if (!Number.isInteger(quantity) || quantity < 0 || quantity > item.quantity) {
      throw new Error(`Exchange between 0 and ${item.quantity} of ${item.title}.`);
    }
    return [{ order_item_id: orderItemId(item), quantity }];
  });
  if (inbound.length === 0) {
    throw new Error("Choose at least one item to exchange.");
  }
  return {
    reason: reason.trim().length > 0 ? reason.trim() : null,
    ...(reasonId.length > 0 ? { reason_id: reasonId } : {}),
    metadata: {},
    inbound_items: inbound,
    outbound_items: inbound.map((line) => ({
      ...line,
      replacement_variant_id: variantId,
    })),
  };
}

/** Kind of post-purchase record the workspace can create. */
export type CreationKind = "edit" | "return" | "claim" | "exchange";

/** Record types that settle a price difference. */
export type DifferenceKind = "edit" | "claim" | "exchange";

export const CREATION_LABELS: Record<CreationKind, string> = {
  edit: "Edit items",
  return: "Create return",
  claim: "Create claim",
  exchange: "Create exchange",
};

export const CREATION_HINTS: Record<CreationKind, string> = {
  edit: "Change quantities or remove lines. The edit is applied only after you confirm it.",
  return: "Choose the units coming back. Requesting a return does not refund it.",
  claim: "Choose the units being claimed. This creates a refund-type claim.",
  exchange: "Choose the units coming back and the replacement variant that ships instead.",
};

/** Opens a payable for the difference an accepted record leaves the customer owing. */
export function collectDifference(
  client: AdminClient,
  orderId: string,
  kind: DifferenceKind,
  recordId: string,
  provider: string,
): Promise<unknown> {
  switch (kind) {
    case "edit":
      return orderAfterSales.collectEditDifference(client, orderId, recordId, provider);
    case "claim":
      return orderAfterSales.collectClaimDifference(client, orderId, recordId, provider);
    case "exchange":
      return orderAfterSales.collectExchangeDifference(client, orderId, recordId, provider);
  }
}

/** Refunds the difference an accepted record owes the customer. */
export function refundDifference(
  client: AdminClient,
  orderId: string,
  kind: DifferenceKind,
  recordId: string,
): Promise<unknown> {
  switch (kind) {
    case "edit":
      return orderAfterSales.refundEditDifference(client, orderId, recordId);
    case "claim":
      return orderAfterSales.refundClaimDifference(client, orderId, recordId);
    case "exchange":
      return orderAfterSales.refundExchangeDifference(client, orderId, recordId);
  }
}

/** Lifecycle actions offered by the drawer, grouped by record type. */
export const orderAfterSales = {
  requestReturn: (client: AdminClient, orderId: string, returnId: string) =>
    client.orders.returns.request(orderId, returnId),
  receiveReturn: (client: AdminClient, orderId: string, returnId: string) =>
    client.orders.returns.receive(orderId, returnId, {}),
  refundReturn: (client: AdminClient, orderId: string, returnId: string) =>
    client.orders.returns.refund(orderId, returnId, {}),
  writeOffReturn: (client: AdminClient, orderId: string, returnId: string) =>
    client.orders.returns.writeOff(orderId, returnId, {}),
  cancelReturn: (client: AdminClient, orderId: string, returnId: string) =>
    client.orders.returns.cancel(orderId, returnId, {}),

  confirmEdit: (client: AdminClient, orderId: string, editId: string) =>
    client.orders.edits.confirm(orderId, editId),
  collectEditDifference: (client: AdminClient, orderId: string, editId: string, provider: string) =>
    client.orders.edits.collectDifference(orderId, editId, { provider }),
  refundEditDifference: (client: AdminClient, orderId: string, editId: string) =>
    client.orders.edits.refundDifference(orderId, editId),
  cancelEdit: (client: AdminClient, orderId: string, editId: string) =>
    client.orders.edits.cancel(orderId, editId, {}),

  acceptClaim: (client: AdminClient, orderId: string, claimId: string) =>
    client.orders.claims.accept(orderId, claimId),
  fulfillClaim: (client: AdminClient, orderId: string, claimId: string) =>
    client.orders.claims.fulfill(orderId, claimId, {}),
  refundClaim: (client: AdminClient, orderId: string, claimId: string) =>
    client.orders.claims.refund(orderId, claimId, {}),
  collectClaimDifference: (
    client: AdminClient,
    orderId: string,
    claimId: string,
    provider: string,
  ) => client.orders.claims.collectDifference(orderId, claimId, { provider }),
  refundClaimDifference: (client: AdminClient, orderId: string, claimId: string) =>
    client.orders.claims.refundDifference(orderId, claimId),
  cancelClaim: (client: AdminClient, orderId: string, claimId: string) =>
    client.orders.claims.cancel(orderId, claimId, {}),

  acceptExchange: (client: AdminClient, orderId: string, exchangeId: string) =>
    client.orders.exchanges.accept(orderId, exchangeId),
  fulfillExchange: (client: AdminClient, orderId: string, exchangeId: string) =>
    client.orders.exchanges.fulfill(orderId, exchangeId, {}),
  collectExchangeDifference: (
    client: AdminClient,
    orderId: string,
    exchangeId: string,
    provider: string,
  ) => client.orders.exchanges.collectDifference(orderId, exchangeId, { provider }),
  refundExchangeDifference: (client: AdminClient, orderId: string, exchangeId: string) =>
    client.orders.exchanges.refundDifference(orderId, exchangeId),
  cancelExchange: (client: AdminClient, orderId: string, exchangeId: string) =>
    client.orders.exchanges.cancel(orderId, exchangeId, {}),
};
