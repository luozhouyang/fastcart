import type { AdminClient } from "@fastcart/admin-sdk";
import { ApiError } from "@fastcart/api-client";
import type {
  Customer,
  Inquiry,
  InquiryFile,
  InquiryForm,
  InquiryFormField,
  InquiryFormRevision,
  InquiryFormSettings,
  InquiryListQuery,
  InquiryStatus,
  InventoryItem,
  InventoryLevel,
  InventoryLocation,
  JsonValue,
  Metadata,
  NotificationChannel,
  NotificationChannelDelivery,
  NotificationChannelEvent,
  Order,
  OrderStatus,
  Payment,
  Product,
  Store,
  Variant,
} from "@fastcart/shared-types";

import { API_BASE_URL, errorMessage, fetchAll } from "@/lib/api";
import { fromUnixSeconds } from "@/lib/format";
import {
  ORDER_STATUS_LABELS,
  swatchFor,
  toMajor,
  type AttentionItemView,
  type CollectionView,
  type CustomerStatusView,
  type CustomerView,
  type CustomersPageView,
  type DomainView,
  type InquiriesPageView,
  type InquiryDetailView,
  type InquiryFormEditorView,
  type InquiryFormsView,
  type InventoryLocationView,
  type InventoryPageView,
  type InventoryRowView,
  type MetricView,
  type NotificationsPageView,
  type OrderHistoryView,
  type OrderItemView,
  type OrderStatusView,
  type OrdersPageView,
  type OrderView,
  type OverviewView,
  type ProductView,
  type ProductsPageView,
  type RevenuePointView,
  type RevenueRange,
  type StoreProfileView,
} from "@/lib/models";

const STATUSES: OrderStatus[] = [
  "pending",
  "confirmed",
  "processing",
  "fulfilled",
  "completed",
  "canceled",
];

export type OrderSort = "newest" | "oldest" | "total";

export interface OrdersQuery {
  status: OrderStatusView | "all";
  page: number;
  pageSize: number;
  query: string;
  range: "all" | RevenueRange;
  sort: OrderSort;
}

/* ------------------------------------------------------------------ */
/*  Mapping helpers                                                    */
/* ------------------------------------------------------------------ */

function customerName(customer: Customer | undefined, email: string): string {
  if (customer) {
    const name = [customer.first_name, customer.last_name].filter(Boolean).join(" ").trim();
    if (name.length > 0) return name;
  }
  const local = email.split("@")[0] ?? email;
  const words = local.split(/[._-]+/).filter(Boolean);
  if (words.length === 0) return email;
  return words.map((word) => word.charAt(0).toUpperCase() + word.slice(1)).join(" ");
}

function addressString(value: JsonValue): string {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return "No shipping address on file";
  }
  const field = (key: string): string => {
    const entry = value[key];
    return typeof entry === "string" ? entry.trim() : "";
  };
  const street = [field("address_1"), field("address_2")].filter(Boolean).join(", ");
  const city = [field("city"), field("province"), field("postal_code")].filter(Boolean).join(" ");
  const country = field("country_code");
  const address = [street, city, country].filter(Boolean).join(", ");
  return address.length > 0 ? address : "No shipping address on file";
}

function metadataString(metadata: Metadata, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = metadata[key];
    if (typeof value === "string" && value.trim().length > 0) return value.trim();
  }
  return null;
}

function describeHistory(order: Order): OrderHistoryView[] {
  const history: OrderHistoryView[] = [];
  const statuses = [...order.status_history].sort((a, b) => a.created_at - b.created_at);

  statuses.forEach((entry, index) => {
    const tone: OrderHistoryView["tone"] =
      entry.status === "canceled"
        ? "muted"
        : entry.status === "pending" || entry.status === "processing"
          ? "warning"
          : "brand";
    history.push({
      label: index === 0 ? "Order placed" : ORDER_STATUS_LABELS[entry.status],
      at: fromUnixSeconds(entry.created_at),
      tone,
    });
  });

  if (history.length === 0) {
    history.push({
      label: "Order placed",
      at: fromUnixSeconds(order.created_at),
      tone: "brand",
    });
  }

  if (
    (order.payment_status === "captured" ||
      order.payment_status === "partially_refunded" ||
      order.payment_status === "refunded") &&
    !history.some((entry) => entry.label.toLowerCase().includes("payment"))
  ) {
    const at = history[0]?.at ?? fromUnixSeconds(order.created_at);
    history.splice(1, 0, { label: "Payment captured", at, tone: "brand" });
  }

  if (
    order.fulfillment_status === "not_fulfilled" &&
    order.status !== "canceled" &&
    order.status !== "completed"
  ) {
    history.push({
      label: "Awaiting fulfillment",
      at: fromUnixSeconds(order.updated_at),
      tone: "warning",
    });
  }

  return history;
}

function orderItems(order: Order): OrderItemView[] {
  return order.items.map((item) => ({
    title: item.product_title,
    variant: item.variant_title ?? "Default",
    sku: item.sku ?? "—",
    quantity: item.quantity,
    price: toMajor(item.unit_price),
  }));
}

/** A display-ready order. Customer names are resolved from a lookup when present. */
export function orderToView(
  order: Order,
  names: Map<string, string>,
  emailNames: Map<string, string>,
): OrderView {
  const name =
    (order.customer_id ? names.get(order.customer_id) : undefined) ??
    emailNames.get(order.email.toLowerCase()) ??
    customerName(undefined, order.email);

  const refundedTotal = toMajor(order.refunds.reduce((total, refund) => total + refund.amount, 0));
  const canFulfill =
    order.status !== "canceled" &&
    order.status !== "completed" &&
    order.status !== "fulfilled" &&
    order.fulfillment_status !== "fulfilled" &&
    order.fulfillment_status !== "returned";
  const canRefund =
    (order.payment_status === "captured" || order.payment_status === "partially_refunded") &&
    refundedTotal < toMajor(order.total);

  return {
    id: order.id,
    number: order.display_number,
    customer: name,
    email: order.email,
    placedAt: fromUnixSeconds(order.created_at),
    status: order.status,
    paymentStatus: order.payment_status,
    fulfillmentStatus: order.fulfillment_status,
    total: toMajor(order.total),
    shipping: toMajor(order.shipping_total),
    tax: toMajor(order.tax_total),
    refundedTotal,
    currency: order.currency_code,
    address: addressString(order.shipping_address),
    note: metadataString(order.metadata, "customer_note", "note"),
    paymentMethod: null,
    shippingMethod: null,
    items: orderItems(order),
    history: describeHistory(order),
    canFulfill,
    canRefund,
  };
}

type Lookup = {
  names: Map<string, string>;
  emailNames: Map<string, string>;
};

function customerLookup(customers: Customer[]): Lookup {
  const names = new Map<string, string>();
  const emailNames = new Map<string, string>();
  for (const customer of customers) {
    const name = customerName(customer, customer.email);
    names.set(customer.id, name);
    emailNames.set(customer.email.toLowerCase(), name);
  }
  return { names, emailNames };
}

function dayKey(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function revenueSeries(orders: Order[], days: number): RevenuePointView[] {
  const buckets = new Map<string, number>();
  const today = new Date();
  for (let index = days - 1; index >= 0; index -= 1) {
    const date = new Date(today);
    date.setDate(today.getDate() - index);
    buckets.set(dayKey(date), 0);
  }
  for (const order of orders) {
    if (order.status === "canceled") continue;
    const key = dayKey(new Date(order.created_at * 1000));
    if (buckets.has(key)) {
      buckets.set(key, (buckets.get(key) ?? 0) + toMajor(order.total));
    }
  }
  return [...buckets.entries()].map(([key, value]) => {
    const date = new Date(`${key}T12:00:00`);
    return {
      date: date.toLocaleDateString("en-US", { month: "short", day: "numeric" }),
      value: Math.round(value * 100) / 100,
    };
  });
}

function moneyMetric(
  key: string,
  label: string,
  value: number,
  delta: number,
  comparison: string,
  display = false,
  format: "money" | "moneyWhole" = "money",
): MetricView {
  return { key, label, value, format, delta, comparison, display };
}

async function orderCounts(
  client: AdminClient,
  signal: AbortSignal,
): Promise<Record<string, number>> {
  const [all, ...statusCounts] = await Promise.all([
    client.orders.list({ limit: 1 }, { signal }),
    ...STATUSES.map((status) => client.orders.list({ limit: 1, status }, { signal })),
  ]);
  const counts: Record<string, number> = { all: all.pagination.total };
  STATUSES.forEach((status, index) => {
    counts[status] = statusCounts[index]?.pagination.total ?? 0;
  });
  return counts;
}

/* ------------------------------------------------------------------ */
/*  Inventory snapshot                                                 */
/* ------------------------------------------------------------------ */

interface InventorySnapshot {
  locations: InventoryLocation[];
  items: InventoryItem[];
  levels: InventoryLevel[];
  products: Product[];
  variants: Map<string, Variant>;
  productsByVariant: Map<string, Product>;
  itemById: Map<string, InventoryItem>;
}

async function inventorySnapshot(
  client: AdminClient,
  signal: AbortSignal,
): Promise<InventorySnapshot> {
  const [locations, items, products] = await Promise.all([
    fetchAll((query) => client.inventory.locations.list(query, { signal }), 200),
    fetchAll((query) => client.inventory.items.list(query, { signal }), 1000),
    fetchAll((query) => client.products.list(query, { signal }), 1000),
  ]);

  const levels = (
    await Promise.all(
      locations.map((location) =>
        client.inventory.levels({ location_id: location.id }, { signal }).catch(() => []),
      ),
    )
  ).flat();

  const variants = new Map<string, Variant>();
  const productsByVariant = new Map<string, Product>();
  for (const product of products) {
    for (const variant of product.variants) {
      variants.set(variant.id, variant);
      productsByVariant.set(variant.id, product);
    }
  }

  return {
    locations,
    items,
    levels,
    products,
    variants,
    productsByVariant,
    itemById: new Map(items.map((item) => [item.id, item])),
  };
}

function levelAvailable(level: InventoryLevel): number {
  return level.stocked_quantity - level.reserved_quantity;
}

/* ------------------------------------------------------------------ */
/*  Overview                                                           */
/* ------------------------------------------------------------------ */

export async function loadOverview(
  client: AdminClient,
  store: Store,
  signal: AbortSignal,
): Promise<OverviewView> {
  const [counts, ordersPage, customers, inventory] = await Promise.all([
    orderCounts(client, signal),
    fetchAll((query) => client.orders.list(query, { signal }), 500),
    fetchAll((query) => client.customers.list(query, { signal }), 300),
    inventorySnapshot(client, signal),
  ]);

  const orders = ordersPage;
  const lookup = customerLookup(customers);
  const now = Date.now();
  const within = (order: Order, fromDays: number, toDays: number): boolean => {
    const at = order.created_at * 1000;
    return (
      order.status !== "canceled" &&
      at >= now - fromDays * 86_400_000 &&
      at < now - toDays * 86_400_000
    );
  };
  const orders30 = orders.filter((order) => within(order, 30, 0));
  const previousOrders30 = orders.filter((order) => within(order, 60, 30));
  const revenue30 = orders30.reduce((total, order) => total + toMajor(order.total), 0);
  const previousRevenue30 = previousOrders30.reduce(
    (total, order) => total + toMajor(order.total),
    0,
  );
  const revenueDelta =
    previousRevenue30 > 0
      ? ((revenue30 - previousRevenue30) / previousRevenue30) * 100
      : revenue30 > 0
        ? 100
        : 0;
  const orderDelta =
    previousOrders30.length > 0
      ? ((orders30.length - previousOrders30.length) / previousOrders30.length) * 100
      : orders30.length > 0
        ? 100
        : 0;
  const orderCount30 = orders30.length;
  const aov = orderCount30 > 0 ? revenue30 / orderCount30 : 0;
  const previousAov = previousOrders30.length > 0 ? previousRevenue30 / previousOrders30.length : 0;
  const aovDelta = previousAov > 0 ? ((aov - previousAov) / previousAov) * 100 : aov > 0 ? 100 : 0;

  const byEmail = new Map<string, number>();
  for (const order of orders30) {
    byEmail.set(order.email.toLowerCase(), (byEmail.get(order.email.toLowerCase()) ?? 0) + 1);
  }
  const withOrders = [...byEmail.values()].filter((value) => value > 0);
  const returning =
    withOrders.length > 0
      ? (withOrders.filter((value) => value > 1).length / withOrders.length) * 100
      : 0;

  const metrics: MetricView[] = [
    moneyMetric(
      "revenue",
      "Net revenue",
      revenue30,
      revenueDelta,
      "vs previous 30 days",
      true,
      "moneyWhole",
    ),
    {
      key: "orders",
      label: "Orders",
      value: orderCount30,
      format: "number",
      delta: orderDelta,
      comparison: "vs previous 30 days",
    },
    moneyMetric("aov", "Average order value", aov, aovDelta, "vs previous 30 days"),
    {
      key: "returning",
      label: "Returning customers",
      value: returning,
      format: "percent",
      delta: 0,
      comparison: "of customers with orders",
    },
  ];

  const sliceOrder: OrderStatus[] = [
    "completed",
    "fulfilled",
    "processing",
    "confirmed",
    "pending",
    "canceled",
  ];
  const sliceColors: Record<OrderStatus, string> = {
    completed: "var(--chart-1)",
    fulfilled: "var(--chart-2)",
    processing: "var(--chart-3)",
    confirmed: "var(--chart-2)",
    pending: "var(--chart-5)",
    canceled: "var(--chart-4)",
  };
  const statusSlices = sliceOrder
    .map((status) => ({
      label: ORDER_STATUS_LABELS[status],
      value: counts[status] ?? 0,
      color: sliceColors[status],
    }))
    .filter((slice) => slice.value > 0);

  const locationById = new Map(inventory.locations.map((location) => [location.id, location]));
  const lowStock = inventory.levels
    .filter((level) => levelAvailable(level) <= 10)
    .sort((a, b) => levelAvailable(a) - levelAvailable(b))
    .slice(0, 2)
    .map((level): AttentionItemView => {
      const item = inventory.itemById.get(level.inventory_item_id);
      const variantId = item?.variant_id ?? "";
      const product = inventory.productsByVariant.get(variantId);
      const available = levelAvailable(level);
      return {
        title: product?.title ?? item?.sku ?? "Inventory item",
        detail: `${available} left at ${locationById.get(level.location_id)?.name ?? "a location"}`,
        value: String(available),
        tone: available <= 0 ? "destructive" : "warning",
        progress: Math.max(0.04, Math.min(1, available / Math.max(level.stocked_quantity, 1))),
      };
    });

  const pending = counts.pending ?? 0;
  const attention: AttentionItemView[] = [...lowStock];
  if (pending > 0) {
    attention.push({
      title: `${pending} ${pending === 1 ? "order" : "orders"} awaiting payment`,
      detail: "Payment has not been captured yet",
      value: String(pending),
      tone: "info",
      progress: Math.min(1, pending / Math.max(counts.all ?? 1, 1)),
    });
  }

  const statusTotal = STATUSES.reduce((total, status) => total + (counts[status] ?? 0), 0);

  return {
    storeName: store.name,
    metrics,
    revenueRanges: {
      "7d": revenueSeries(orders, 7),
      "30d": revenueSeries(orders, 30),
      "90d": revenueSeries(orders, 90),
    },
    revenueDelta,
    revenueComparison: "vs previous 30 days",
    statusSlices,
    totalOrders: statusTotal,
    recentOrders: orders
      .slice(0, 5)
      .map((order) => orderToView(order, lookup.names, lookup.emailNames)),
    attention,
    lastSynced: new Date().toISOString(),
  };
}

/* ------------------------------------------------------------------ */
/*  Orders                                                             */
/* ------------------------------------------------------------------ */

export async function loadOrders(
  client: AdminClient,
  query: OrdersQuery,
  signal: AbortSignal,
): Promise<OrdersPageView> {
  const [counts, customers] = await Promise.all([
    orderCounts(client, signal),
    fetchAll((page) => client.customers.list(page, { signal }), 300),
  ]);
  const lookup = customerLookup(customers);
  const needle = query.query.trim();
  const filteredLocally = needle.length > 0 || query.range !== "all" || query.sort !== "newest";

  let orders: Order[];
  let total: number;

  const rangeDays =
    query.range === "7d" ? 7 : query.range === "30d" ? 30 : query.range === "90d" ? 90 : null;
  const cutoff = rangeDays === null ? null : Date.now() - rangeDays * 86_400_000;

  if (/^\d+$/.test(needle)) {
    try {
      const order = await client.orders.getByNumber(Number(needle), { signal });
      const matchesStatus = query.status === "all" || order.status === query.status;
      const matchesRange = cutoff === null || order.created_at * 1000 >= cutoff;
      orders = matchesStatus && matchesRange ? [order] : [];
      total = orders.length;
    } catch (cause) {
      if (!(cause instanceof ApiError && cause.status === 404)) throw cause;
      orders = [];
      total = 0;
    }
  } else if (filteredLocally) {
    const sortedView = needle.length === 0 && cutoff === null && query.sort !== "newest";
    const source = sortedView
      ? await fetchAll(
          (page) =>
            client.orders.list(
              {
                ...page,
                ...(query.status === "all" ? {} : { status: query.status }),
              },
              { signal },
            ),
          500,
        )
      : (
          await client.orders.list(
            {
              limit: 100,
              ...(query.status === "all" ? {} : { status: query.status }),
            },
            { signal },
          )
        ).data;
    const lowered = needle.toLowerCase();

    let filtered = source.filter((order) => {
      if (cutoff !== null && order.created_at * 1000 < cutoff) return false;
      if (lowered.length === 0) return true;
      const name =
        (order.customer_id ? lookup.names.get(order.customer_id) : undefined) ??
        lookup.emailNames.get(order.email.toLowerCase()) ??
        "";
      return (
        name.toLowerCase().includes(lowered) ||
        order.email.toLowerCase().includes(lowered) ||
        String(order.display_number).includes(lowered)
      );
    });

    if (query.sort === "oldest") {
      filtered = filtered.sort((a, b) => a.created_at - b.created_at);
    } else if (query.sort === "total") {
      filtered = filtered.sort((a, b) => b.total - a.total);
    }

    total = filtered.length;
    const offset = (query.page - 1) * query.pageSize;
    orders = filtered.slice(offset, offset + query.pageSize);
  } else {
    const page = await client.orders.list(
      {
        limit: query.pageSize,
        offset: (query.page - 1) * query.pageSize,
        ...(query.status === "all" ? {} : { status: query.status }),
      },
      { signal },
    );
    orders = page.data;
    total = page.pagination.total;
  }

  return {
    orders: orders.map((order) => orderToView(order, lookup.names, lookup.emailNames)),
    total,
    loaded: orders.length,
    counts,
    status: query.status,
  };
}

/* ------------------------------------------------------------------ */
/*  Products                                                           */
/* ------------------------------------------------------------------ */

export async function loadProducts(
  client: AdminClient,
  store: Store,
  signal: AbortSignal,
): Promise<ProductsPageView> {
  const [products, prices, inventory] = await Promise.all([
    fetchAll((query) => client.products.list(query, { signal }), 1000),
    fetchAll((query) => client.priceLists.prices(query, { signal }), 1000),
    inventorySnapshot(client, signal),
  ]);

  const priceByVariant = new Map<string, number>();
  for (const price of prices) {
    if (price.currency_code !== store.currency_code && priceByVariant.has(price.variant_id)) {
      continue;
    }
    priceByVariant.set(price.variant_id, toMajor(price.amount));
  }

  const stockByVariant = new Map<string, number>();
  for (const level of inventory.levels) {
    const item = inventory.itemById.get(level.inventory_item_id);
    if (!item?.variant_id) continue;
    stockByVariant.set(
      item.variant_id,
      (stockByVariant.get(item.variant_id) ?? 0) + level.stocked_quantity,
    );
  }

  const views: ProductView[] = products.map((product) => {
    const price =
      product.variants
        .map((entry) => priceByVariant.get(entry.id))
        .find((value) => value !== undefined) ?? null;
    const stock = product.variants.reduce(
      (total, entry) => total + (stockByVariant.get(entry.id) ?? 0),
      0,
    );
    return {
      id: product.id,
      title: product.title,
      handle: product.handle,
      collection: product.collections[0]?.title ?? "Uncategorized",
      price,
      stock,
      status: product.status,
      updatedAt: fromUnixSeconds(product.updated_at),
      swatch: swatchFor(product.id),
    };
  });

  const collectionCounts = new Map<string, ProductView[]>();
  for (const product of views) {
    const list = collectionCounts.get(product.collection) ?? [];
    list.push(product);
    collectionCounts.set(product.collection, list);
  }
  const collections: CollectionView[] = [...collectionCounts.entries()]
    .sort((a, b) => b[1].length - a[1].length)
    .slice(0, 4)
    .map(([title, list]) => ({
      title,
      count: list.length,
      colors: list.slice(0, 3).map((product) => product.swatch[0]),
    }));

  return {
    products: views,
    collections,
    counts: {
      all: views.length,
      active: views.filter((product) => product.status === "active").length,
      draft: views.filter((product) => product.status === "draft").length,
      archived: views.filter((product) => product.status === "archived").length,
    },
  };
}

/* ------------------------------------------------------------------ */
/*  Inventory                                                          */
/* ------------------------------------------------------------------ */

export async function loadInventory(
  client: AdminClient,
  signal: AbortSignal,
): Promise<InventoryPageView> {
  const inventory = await inventorySnapshot(client, signal);
  const locationById = new Map(inventory.locations.map((location) => [location.id, location]));

  const rows: InventoryRowView[] = inventory.levels
    .map((level): InventoryRowView => {
      const item = inventory.itemById.get(level.inventory_item_id);
      const variantId = item?.variant_id ?? "";
      const product = inventory.productsByVariant.get(variantId);
      return {
        id: level.id,
        inventoryItemId: level.inventory_item_id,
        locationId: level.location_id,
        product: product?.title ?? "Unknown product",
        sku: item?.sku ?? inventory.variants.get(variantId)?.sku ?? "—",
        location: locationById.get(level.location_id)?.name ?? "Unknown location",
        stocked: level.stocked_quantity,
        reserved: level.reserved_quantity,
        incoming: level.incoming_quantity,
        swatch: swatchFor(variantId || level.inventory_item_id),
      };
    })
    .sort((a, b) => a.location.localeCompare(b.location) || a.product.localeCompare(b.product));

  const locations: InventoryLocationView[] = inventory.locations.map((location) => {
    const levels = inventory.levels.filter((level) => level.location_id === location.id);
    const address = [
      metadataString(location.address, "address_1"),
      metadataString(location.address, "city"),
      metadataString(location.address, "province"),
    ]
      .filter(Boolean)
      .join(", ");
    return {
      id: location.id,
      name: location.name,
      address:
        address.length > 0
          ? address
          : (metadataString(location.metadata, "address") ?? "No address recorded"),
      units: levels.reduce((total, level) => total + level.stocked_quantity, 0),
      items: new Set(levels.map((level) => level.inventory_item_id)).size,
      primary: location.id === inventory.locations[0]?.id,
    };
  });

  const totalUnits = rows.reduce((total, row) => total + row.stocked, 0);
  const lowCount = rows.filter((row) => row.stocked - row.reserved <= 10).length;

  return { locations, rows, totalUnits, lowCount };
}

/* ------------------------------------------------------------------ */
/*  Customers                                                          */
/* ------------------------------------------------------------------ */

function customerStatus(customer: Customer): CustomerStatusView {
  if (customer.status === "disabled") return "disabled";
  if (!customer.has_account) return "guest";
  const joined = customer.created_at * 1000;
  return joined >= Date.now() - 30 * 86_400_000 ? "new" : "active";
}

export async function loadCustomers(
  client: AdminClient,
  signal: AbortSignal,
): Promise<CustomersPageView> {
  const [customers, orders, totalPage] = await Promise.all([
    fetchAll((query) => client.customers.list(query, { signal }), 1000),
    fetchAll((query) => client.orders.list(query, { signal }), 500),
    client.customers.list({ limit: 1 }, { signal }),
  ]);

  const byCustomer = new Map<string, { orders: number; spent: number; lastOrder: number }>();
  const byEmail = new Map<string, { orders: number; spent: number; lastOrder: number }>();
  for (const order of orders) {
    if (order.status === "canceled") continue;
    const entry = {
      orders: 1,
      spent: toMajor(order.total),
      lastOrder: order.created_at,
    };
    const merge = (current: { orders: number; spent: number; lastOrder: number } | undefined) => ({
      orders: (current?.orders ?? 0) + entry.orders,
      spent: (current?.spent ?? 0) + entry.spent,
      lastOrder: Math.max(current?.lastOrder ?? 0, entry.lastOrder),
    });
    if (order.customer_id)
      byCustomer.set(order.customer_id, merge(byCustomer.get(order.customer_id)));
    byEmail.set(order.email.toLowerCase(), merge(byEmail.get(order.email.toLowerCase())));
  }

  const views: CustomerView[] = customers.map((customer) => {
    const stats = byCustomer.get(customer.id) ?? byEmail.get(customer.email.toLowerCase());
    return {
      id: customer.id,
      name: customerName(customer, customer.email),
      email: customer.email,
      status: customerStatus(customer),
      orders: stats?.orders ?? 0,
      spent: stats?.spent ?? 0,
      lastOrder: fromUnixSeconds(stats?.lastOrder ?? customer.created_at),
      joined: fromUnixSeconds(customer.created_at),
    };
  });

  const ordersWithCustomer = views.filter((view) => view.orders > 0);
  const totalOrders = ordersWithCustomer.reduce((total, view) => total + view.orders, 0);
  const totalSpent = ordersWithCustomer.reduce((total, view) => total + view.spent, 0);
  const returning =
    ordersWithCustomer.length > 0
      ? (ordersWithCustomer.filter((view) => view.orders > 1).length / ordersWithCustomer.length) *
        100
      : 0;

  const startOfMonth = new Date();
  startOfMonth.setDate(1);
  startOfMonth.setHours(0, 0, 0, 0);
  const newThisMonth = customers.filter(
    (customer) => customer.created_at * 1000 >= startOfMonth.getTime(),
  ).length;

  return {
    customers: views,
    total: totalPage.pagination.total,
    metrics: [
      {
        key: "total",
        label: "Total customers",
        value: totalPage.pagination.total,
        format: "number",
        delta: 0,
        comparison: "all time",
      },
      {
        key: "new",
        label: "New this month",
        value: newThisMonth,
        format: "number",
        delta: 0,
        comparison: "since the first of the month",
      },
      {
        key: "spend",
        label: "Average spend",
        value: totalOrders > 0 ? totalSpent / totalOrders : 0,
        format: "money",
        delta: 0,
        comparison: "per order",
      },
      {
        key: "returning",
        label: "Returning customers",
        value: returning,
        format: "percent",
        delta: 0,
        comparison: "of customers with orders",
      },
    ],
  };
}

/* ------------------------------------------------------------------ */
/*  Settings                                                           */
/* ------------------------------------------------------------------ */

export async function loadSettings(
  client: AdminClient,
  store: Store,
  signal: AbortSignal,
): Promise<StoreProfileView> {
  const [settings, domains] = await Promise.all([
    client.settings.get({ signal }).catch(() => ({}) as Record<string, string>),
    client.stores.domains.list(store.id, { signal }).catch(() => []),
  ]);

  const domainViews: DomainView[] = domains.map((domain) => ({
    id: domain.id,
    hostname: domain.hostname,
    primary: domain.is_primary,
  }));

  return {
    storeId: store.id,
    name: store.name,
    handle: store.slug,
    supportEmail: settings.support_email ?? "",
    currency: store.currency_code,
    timezone: store.timezone,
    weightUnit: settings.weight_unit ?? "kg",
    primaryDomain:
      store.primary_domain ?? domains.find((domain) => domain.is_primary)?.hostname ?? "",
    domains: domainViews,
    checkout: {
      guestCheckout: settings.guest_checkout !== "false",
      requirePhone: settings.require_phone === "true",
      confirmationEmails: settings.confirmation_emails !== "false",
    },
  };
}

/* ------------------------------------------------------------------ */
/*  Mutations                                                          */
/* ------------------------------------------------------------------ */

export async function fulfillOrder(
  client: AdminClient,
  orderId: string,
  tracking: string,
): Promise<void> {
  await client.orders.fulfill(orderId, {
    tracking_numbers: tracking.trim().length > 0 ? [tracking.trim()] : [],
  });
}

export async function refundOrder(
  client: AdminClient,
  order: OrderView,
  amount: number,
  reason: string,
): Promise<void> {
  const payments = await client.payments.list({ order_id: order.id, limit: 10 });
  const payment = payments.data.find((entry: Payment) => entry.order_id === order.id);
  if (!payment) {
    throw new Error("No payment is recorded for this order.");
  }
  await client.orders.refunds.create(order.id, {
    payment_id: payment.id,
    amount: Math.round(amount * 100),
    currency: order.currency,
    ...(reason.trim().length > 0 ? { reason: reason.trim() } : {}),
  });
}

export async function adjustStock(
  client: AdminClient,
  row: InventoryRowView,
  delta: number,
  reason: string,
): Promise<void> {
  await client.inventory.adjust({
    inventory_item_id: row.inventoryItemId,
    location_id: row.locationId,
    quantity_delta: delta,
    reason,
    reference: "admin-ui",
  });
}

export async function createProduct(
  client: AdminClient,
  input: { title: string; sku: string; description: string },
): Promise<Product> {
  const variant: { title: string; sku?: string } = { title: "Default" };
  if (input.sku.trim().length > 0) variant.sku = input.sku.trim();
  return client.products.create({
    title: input.title.trim(),
    ...(input.description.trim().length > 0 ? { description: input.description.trim() } : {}),
    variants: [variant],
  });
}

export async function saveStoreProfile(
  client: AdminClient,
  store: Store,
  profile: StoreProfileView,
): Promise<void> {
  await client.stores.update(store.id, {
    name: profile.name.trim(),
    currency_code: profile.currency,
    timezone: profile.timezone,
    locale: store.locale,
  });
  await client.settings.update({
    values: {
      support_email: profile.supportEmail.trim(),
      weight_unit: profile.weightUnit,
      guest_checkout: String(profile.checkout.guestCheckout),
      require_phone: String(profile.checkout.requirePhone),
      confirmation_emails: String(profile.checkout.confirmationEmails),
    },
  });
}

export async function addDomain(
  client: AdminClient,
  storeId: string,
  hostname: string,
  primary = false,
): Promise<void> {
  await client.stores.domains.add(storeId, { hostname: hostname.trim(), is_primary: primary });
}

export async function setPrimaryDomain(
  client: AdminClient,
  storeId: string,
  hostname: string,
): Promise<void> {
  await client.stores.domains.setPrimary(storeId, { hostname });
}

/* ------------------------------------------------------------------ */
/*  Inquiry forms                                                      */
/* ------------------------------------------------------------------ */

/** Some endpoints answer with a bare array; normalize either envelope. */
function asArray<T>(value: unknown): T[] {
  if (Array.isArray(value)) return value as T[];
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    for (const key of ["data", "items", "events", "revisions", "deliveries"]) {
      const entry = record[key];
      if (Array.isArray(entry)) return entry as T[];
    }
  }
  return [];
}

export async function loadInquiryForms(
  client: AdminClient,
  signal: AbortSignal,
): Promise<InquiryFormsView> {
  const page = await client.inquiryForms.list({ limit: 100 }, { signal });
  const forms = asArray<InquiryForm>(page);
  const record = page as unknown as { pagination?: { total?: number } };
  return {
    forms,
    total: record.pagination?.total ?? forms.length,
  };
}

export async function loadInquiryFormEditor(
  client: AdminClient,
  formId: string,
  signal: AbortSignal,
): Promise<InquiryFormEditorView> {
  const [detail, revisions] = await Promise.all([
    client.inquiryForms.get(formId, { signal }),
    client.inquiryForms.listRevisions(formId, { signal }),
  ]);
  return {
    form: detail.form,
    revisions: asArray<InquiryFormRevision>(revisions).sort(
      (left, right) => right.version - left.version,
    ),
  };
}

export async function createInquiryForm(
  client: AdminClient,
  input: { name: string; handle?: string; description?: string },
): Promise<InquiryForm> {
  return client.inquiryForms.create({
    name: input.name.trim(),
    ...(input.handle && input.handle.trim().length > 0 ? { handle: input.handle.trim() } : {}),
    ...(input.description && input.description.trim().length > 0
      ? { description: input.description.trim() }
      : {}),
  });
}

export async function archiveInquiryForm(
  client: AdminClient,
  formId: string,
): Promise<void> {
  await client.inquiryForms.delete(formId);
}

export async function saveInquiryFormDraft(
  client: AdminClient,
  formId: string,
  fields: InquiryFormField[],
  settings: InquiryFormSettings,
): Promise<InquiryFormRevision> {
  return client.inquiryForms.saveDraft(formId, { fields, settings });
}

export async function publishInquiryForm(
  client: AdminClient,
  formId: string,
): Promise<InquiryForm> {
  return client.inquiryForms.publish(formId);
}

export async function publishInquiryFormVersion(
  client: AdminClient,
  formId: string,
  version: number,
): Promise<InquiryForm> {
  return client.inquiryForms.publishVersion(formId, version);
}

/* ------------------------------------------------------------------ */
/*  Inquiries                                                          */
/* ------------------------------------------------------------------ */

export interface InquiriesQuery {
  status: InquiryStatus | "all";
  formId: string;
  query: string;
  from: string;
  to: string;
  limit: number;
  offset: number;
}

function startOfDaySeconds(value: string): number | undefined {
  if (value.length === 0) return undefined;
  const date = new Date(`${value}T00:00:00`);
  return Number.isNaN(date.getTime()) ? undefined : Math.floor(date.getTime() / 1000);
}

function endOfDaySeconds(value: string): number | undefined {
  if (value.length === 0) return undefined;
  const date = new Date(`${value}T23:59:59`);
  return Number.isNaN(date.getTime()) ? undefined : Math.floor(date.getTime() / 1000);
}

export async function loadInquiries(
  client: AdminClient,
  query: InquiriesQuery,
  signal: AbortSignal,
): Promise<InquiriesPageView> {
  const params: InquiryListQuery = {
    limit: query.limit,
    offset: query.offset,
  };
  if (query.status !== "all") params.status = query.status;
  if (query.formId.length > 0) params.form_id = query.formId;
  if (query.query.trim().length > 0) params.query = query.query.trim();
  const from = startOfDaySeconds(query.from);
  const to = endOfDaySeconds(query.to);
  if (from !== undefined) params.from = from;
  if (to !== undefined) params.to = to;

  const [page, forms] = await Promise.all([
    client.inquiries.list(params, { signal }),
    client.inquiryForms.list({ limit: 100 }, { signal }).catch(() => ({
      data: [] as InquiryForm[],
      pagination: { limit: 0, offset: 0, total: 0 },
    })),
  ]);

  return {
    inquiries: asArray<Inquiry>(page.data),
    total: page.pagination?.total ?? page.data.length,
    forms: asArray<InquiryForm>(forms),
  };
}

export async function loadInquiryDetail(
  client: AdminClient,
  inquiryId: string,
  signal: AbortSignal,
): Promise<InquiryDetailView> {
  const payload = (await client.inquiries.get(inquiryId, { signal })) as unknown;
  const record = (payload ?? {}) as Record<string, unknown>;
  const inquiry = (record.inquiry ?? record) as Inquiry;
  const revision = (record.revision ??
    (inquiry as unknown as Record<string, unknown>).revision ??
    null) as InquiryFormRevision | null;
  const files = asArray<InquiryFile>(
    record.files ?? (inquiry as unknown as Record<string, unknown>).files,
  );
  return { inquiry, revision, files };
}

export async function updateInquiryStatus(
  client: AdminClient,
  inquiryId: string,
  status: InquiryStatus,
): Promise<Inquiry> {
  return client.inquiries.updateStatus(inquiryId, { status });
}

export async function downloadInquiryFile(url: string, filename: string): Promise<void> {
  const target = url.startsWith("/") ? `${API_BASE_URL}${url}` : url;
  const response = await fetch(target, { credentials: "include" });
  if (!response.ok) {
    throw new Error(`The attachment could not be downloaded (${response.status}).`);
  }
  const blob = await response.blob();
  const href = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(href);
}

/* ------------------------------------------------------------------ */
/*  Notification channels                                              */
/* ------------------------------------------------------------------ */

export async function loadNotifications(
  client: AdminClient,
  signal: AbortSignal,
): Promise<NotificationsPageView> {
  const [channels, events] = await Promise.all([
    client.notificationChannels.list({ signal }),
    client.notificationChannels.events({ signal }).catch(() => []),
  ]);
  return {
    channels: asArray<NotificationChannel>(channels),
    events: asArray<NotificationChannelEvent>(events),
  };
}

export async function loadNotificationDeliveries(
  client: AdminClient,
  channelId: string,
  signal: AbortSignal,
): Promise<NotificationChannelDelivery[]> {
  const deliveries = await client.notificationChannels.deliveries(
    channelId,
    { limit: 50 },
    { signal },
  );
  return asArray<NotificationChannelDelivery>(deliveries);
}

export { errorMessage };
