import {
  customers as mockCustomers,
  inventoryRows,
  locations,
  metrics as mockMetrics,
  notifications,
  orderStatusBreakdown,
  orders as mockOrders,
  products as mockProducts,
  revenueSeries,
  totalOrders,
  type MockOrder,
} from "@/data/mock";
import type {
  AttentionItemView,
  CollectionView,
  CustomersPageView,
  InventoryLocationView,
  InventoryPageView,
  OrderHistoryView,
  OrderItemView,
  OrderStatusView,
  OrderView,
  OverviewView,
  ProductView,
  ProductsPageView,
  OrdersPageView,
  RevenueRange,
  StoreProfileView,
} from "@/lib/models";

function historyFor(order: MockOrder): OrderHistoryView[] {
  const history: OrderHistoryView[] = [
    { label: "Order placed", at: order.placedAt, tone: "brand" },
  ];
  if (order.paymentStatus === "captured") {
    history.push({ label: "Payment captured", at: order.placedAt, tone: "brand" });
  }
  if (order.fulfillmentStatus === "fulfilled") {
    history.push({ label: "Fulfilled", at: order.placedAt, tone: "brand" });
  } else if (order.fulfillmentStatus === "returned") {
    history.push({ label: "Returned", at: order.placedAt, tone: "muted" });
  } else {
    history.push({ label: "Awaiting fulfillment", at: order.placedAt, tone: "warning" });
  }
  return history;
}

function itemsFor(order: MockOrder): OrderItemView[] {
  return order.items.map((item) => ({
    title: item.title,
    variant: item.variant,
    sku: item.sku,
    quantity: item.quantity,
    price: item.price,
  }));
}

export function orderView(order: MockOrder): OrderView {
  const canFulfill =
    order.status !== "canceled" &&
    order.status !== "completed" &&
    order.status !== "fulfilled" &&
    order.fulfillmentStatus !== "fulfilled" &&
    order.fulfillmentStatus !== "returned";
  return {
    id: order.id,
    number: order.displayNumber,
    customer: order.customer,
    email: order.email,
    placedAt: order.placedAt,
    status: order.status,
    paymentStatus: order.paymentStatus,
    fulfillmentStatus: order.fulfillmentStatus,
    total: order.total,
    shipping: order.shipping,
    tax: order.tax,
    refundedTotal:
      order.paymentStatus === "refunded"
        ? order.total
        : order.paymentStatus === "partially_refunded"
          ? Math.round(order.total / 2)
          : 0,
    currency: "USD",
    address: order.address,
    note:
      order.displayNumber === 1042 ? "Please leave with the concierge if nobody is home." : null,
    paymentMethod: order.paymentStatus === "awaiting" ? null : "Visa ending 4242",
    shippingMethod: "Standard · 3–5 days",
    items: itemsFor(order),
    history: historyFor(order),
    canFulfill,
    canRefund: order.paymentStatus === "captured" || order.paymentStatus === "partially_refunded",
  };
}

export function demoOverview(): OverviewView {
  const attention: AttentionItemView[] = [
    {
      title: "Arc Table Lamp",
      detail: "6 left at Brooklyn Studio",
      value: "Low",
      tone: "warning",
      progress: 0.3,
    },
    {
      title: "Tide Glass Vessel",
      detail: "Out of stock · draft product",
      value: "0",
      tone: "destructive",
      progress: 0.06,
    },
    {
      title: "4 orders awaiting payment",
      detail: "Oldest waiting for 6 hours",
      value: "4",
      tone: "info",
      progress: 0.18,
    },
  ];

  return {
    storeName: "Halcyon Supply",
    metrics: mockMetrics.map((metric) => ({
      key: metric.key,
      label: metric.label,
      value: metric.value,
      format: metric.key === "revenue" ? ("moneyWhole" as const) : metric.format,
      delta: metric.delta,
      comparison: metric.comparison,
      display: metric.key === "revenue",
    })),
    revenueRanges: revenueSeries,
    revenueDelta: mockMetrics[0]?.delta ?? 0,
    revenueComparison: "vs previous 30 days",
    statusSlices: orderStatusBreakdown,
    totalOrders,
    recentOrders: mockOrders.slice(0, 5).map(orderView),
    attention,
    lastSynced: new Date().toISOString(),
  };
}

function countsForOrders(): Record<string, number> {
  const counts: Record<string, number> = {
    all: totalOrders,
    pending: 164,
    confirmed: 0,
    processing: 214,
    fulfilled: 348,
    completed: 512,
    canceled: 46,
  };
  return counts;
}

export interface OrdersQuery {
  status: OrderStatusView | "all";
  page: number;
  pageSize: number;
  query: string;
  range: "all" | RevenueRange;
  sort: "newest" | "oldest" | "total";
}

const MOCK_NOW = new Date("2026-11-04T23:59:59Z").getTime();

function inRange(iso: string, range: "all" | RevenueRange): boolean {
  if (range === "all") return true;
  const days = range === "7d" ? 7 : range === "30d" ? 30 : 90;
  return new Date(iso).getTime() >= MOCK_NOW - days * 86_400_000;
}

export function demoOrdersPage({
  status,
  page,
  pageSize,
  query,
  range,
  sort,
}: OrdersQuery): OrdersPageView {
  const needle = query.trim().toLowerCase();
  const filtered = mockOrders.filter((order) => {
    if (status !== "all" && order.status !== status) return false;
    if (!inRange(order.placedAt, range)) return false;
    if (needle.length === 0) return true;
    return (
      order.customer.toLowerCase().includes(needle) ||
      order.email.toLowerCase().includes(needle) ||
      String(order.displayNumber).includes(needle)
    );
  });

  const counts = countsForOrders();
  const total = needle.length > 0 ? filtered.length : (counts[status] ?? 0);
  const offset = (page - 1) * pageSize;
  const sorted = [...filtered];
  if (sort === "oldest") sorted.sort((a, b) => a.placedAt.localeCompare(b.placedAt));
  if (sort === "total") sorted.sort((a, b) => b.total - a.total);

  return {
    orders: sorted.slice(offset, offset + pageSize).map(orderView),
    total,
    loaded: filtered.length,
    counts,
    status,
  };
}

export function demoProducts(): ProductsPageView {
  const products: ProductView[] = mockProducts.map((product) => ({
    id: product.id,
    title: product.title,
    handle: product.handle,
    collection: product.collection,
    price: product.price,
    stock: product.stock,
    status: product.status,
    updatedAt: product.updatedAt,
    swatch: product.swatch,
  }));
  const collectionCounts = new Map<string, number>();
  for (const product of mockProducts) {
    collectionCounts.set(product.collection, (collectionCounts.get(product.collection) ?? 0) + 1);
  }
  const collections: CollectionView[] = [...collectionCounts.entries()]
    .slice(0, 4)
    .map(([title, count], index) => ({
      title,
      count,
      colors: mockProducts
        .filter((product) => product.collection === title)
        .slice(0, 3)
        .map(
          (product) => product.swatch[0] ?? ["#A9BFAE", "#D9CBB2", "#CBA48F"][index] ?? "#A9BFAE",
        ),
    }));
  const counts = {
    all: products.length,
    active: products.filter((p) => p.status === "active").length,
    draft: products.filter((p) => p.status === "draft").length,
    archived: products.filter((p) => p.status === "archived").length,
  };
  return { products, collections, counts };
}

export function demoCustomers(): CustomersPageView {
  return {
    customers: mockCustomers.map((customer) => ({
      id: customer.id,
      name: customer.name,
      email: customer.email,
      status: customer.status,
      orders: customer.orders,
      spent: customer.spent,
      lastOrder: customer.lastOrder,
      joined: customer.joined,
    })),
    total: 2480,
    metrics: [
      {
        key: "total",
        label: "Total customers",
        value: 2480,
        format: "number",
        delta: 0,
        comparison: "all time",
      },
      {
        key: "new",
        label: "New this month",
        value: 186,
        format: "number",
        delta: 0,
        comparison: "+38 vs October",
      },
      {
        key: "spend",
        label: "Average spend",
        value: 120.23,
        format: "money",
        delta: 0,
        comparison: "per order",
      },
      {
        key: "returning",
        label: "Returning customers",
        value: 38.6,
        format: "percent",
        delta: 0,
        comparison: "of all orders",
      },
    ],
  };
}

export function demoInventory(): InventoryPageView {
  const locationsView: InventoryLocationView[] = locations.map((location) => ({
    id: location.id,
    name: location.name,
    address: location.address,
    units: location.units,
    items: location.items,
    primary: location.primary,
  }));
  const rows = inventoryRows.map((row, index) => ({
    id: row.id,
    inventoryItemId: row.id,
    locationId: locationsView.find((location) => location.name === row.location)?.id ?? "l-01",
    product: row.product,
    sku: row.sku,
    location: row.location,
    stocked: row.stocked,
    reserved: row.reserved,
    incoming: row.incoming,
    swatch: mockProducts[index % mockProducts.length]?.swatch ?? ["#A9BFAE", "#425D4A"],
  }));
  const totalUnits = rows.reduce((total, row) => total + row.stocked, 0);
  const lowCount = rows.filter((row) => row.stocked - row.reserved <= 10).length;
  return { locations: locationsView, rows, totalUnits, lowCount };
}

export function demoSettings(): StoreProfileView {
  return {
    storeId: "demo-store",
    name: "Halcyon Supply",
    handle: "halcyon-supply",
    supportEmail: "hello@halcyonsupply.com",
    currency: "USD",
    timezone: "America/New_York",
    weightUnit: "kg",
    primaryDomain: "halcyonsupply.com",
    domains: [
      { id: "d-1", hostname: "halcyonsupply.com", primary: true },
      { id: "d-2", hostname: "shop.halcyonsupply.com", primary: false },
    ],
    checkout: {
      guestCheckout: true,
      requirePhone: false,
      confirmationEmails: true,
    },
  };
}

export const demoNotifications = notifications.map((notification) => ({
  id: notification.id,
  title: notification.title,
  body: notification.body,
  time: notification.time,
  tone: notification.tone,
}));
