import type {
  Inquiry,
  InquiryFile,
  InquiryForm,
  InquiryFormRevision,
  NotificationChannel,
  NotificationChannelEvent,
} from "@fastcart/shared-types";

export type OrderStatusView =
  "pending" | "confirmed" | "processing" | "fulfilled" | "completed" | "canceled";

export type PaymentStatusView =
  "awaiting" | "authorized" | "captured" | "partially_refunded" | "refunded" | "failed";

export type FulfillmentStatusView =
  "not_fulfilled" | "partially_fulfilled" | "fulfilled" | "returned";

export type ProductStatusView = "active" | "draft" | "archived";

export type CustomerStatusView = "active" | "new" | "guest" | "disabled";

export interface MetricView {
  key: string;
  label: string;
  value: number;
  format: "money" | "moneyWhole" | "number" | "percent";
  delta: number;
  comparison: string;
  display?: boolean;
}

export interface RevenuePointView {
  date: string;
  value: number;
}

export interface StatusSliceView {
  label: string;
  value: number;
  color: string;
}

export interface AttentionItemView {
  title: string;
  detail: string;
  value: string;
  tone: "warning" | "destructive" | "info";
  progress: number;
}

export interface OrderItemView {
  title: string;
  variant: string;
  sku: string;
  quantity: number;
  price: number;
}

export interface OrderHistoryView {
  label: string;
  at: string;
  tone: "brand" | "warning" | "muted";
}

export interface OrderView {
  id: string;
  number: number;
  customer: string;
  email: string;
  placedAt: string;
  status: OrderStatusView;
  paymentStatus: PaymentStatusView;
  fulfillmentStatus: FulfillmentStatusView;
  total: number;
  shipping: number;
  tax: number;
  refundedTotal: number;
  currency: string;
  address: string;
  note: string | null;
  paymentMethod: string | null;
  shippingMethod: string | null;
  items: OrderItemView[];
  history: OrderHistoryView[];
  canFulfill: boolean;
  canRefund: boolean;
}

export interface ProductView {
  id: string;
  title: string;
  handle: string;
  collection: string;
  price: number | null;
  stock: number;
  status: ProductStatusView;
  updatedAt: string;
  swatch: [string, string];
}

export interface CollectionView {
  title: string;
  count: number;
  colors: string[];
}

export interface CustomerView {
  id: string;
  name: string;
  email: string;
  status: CustomerStatusView;
  orders: number;
  spent: number;
  lastOrder: string;
  joined: string;
}

export interface InventoryLocationView {
  id: string;
  name: string;
  address: string;
  units: number;
  items: number;
  primary: boolean;
}

export interface InventoryRowView {
  id: string;
  inventoryItemId: string;
  locationId: string;
  product: string;
  sku: string;
  location: string;
  stocked: number;
  reserved: number;
  incoming: number;
  swatch: [string, string];
}

export interface OverviewView {
  storeName: string;
  metrics: MetricView[];
  revenueRanges: Record<RevenueRange, RevenuePointView[]>;
  revenueDelta: number;
  revenueComparison: string;
  statusSlices: StatusSliceView[];
  totalOrders: number;
  recentOrders: OrderView[];
  attention: AttentionItemView[];
  lastSynced: string;
}

export type RevenueRange = "7d" | "30d" | "90d";

export interface OrdersPageView {
  orders: OrderView[];
  total: number;
  /** Rows available in demo mode, where the sample is smaller than the headline count. */
  loaded?: number;
  counts: Record<string, number>;
  status: OrderStatusView | "all";
}

export interface ProductsPageView {
  products: ProductView[];
  collections: CollectionView[];
  counts: Record<"all" | ProductStatusView, number>;
}

export interface CustomersPageView {
  customers: CustomerView[];
  total: number;
  metrics: MetricView[];
}

export interface InventoryPageView {
  locations: InventoryLocationView[];
  rows: InventoryRowView[];
  totalUnits: number;
  lowCount: number;
}

export interface DomainView {
  id: string;
  hostname: string;
  primary: boolean;
}

export interface StoreProfileView {
  storeId: string;
  name: string;
  handle: string;
  supportEmail: string;
  currency: string;
  timezone: string;
  weightUnit: string;
  primaryDomain: string;
  domains: DomainView[];
  checkout: {
    guestCheckout: boolean;
    requirePhone: boolean;
    confirmationEmails: boolean;
  };
}

export interface NotificationView {
  id: string;
  title: string;
  body: string;
  time: string;
  tone: "brand" | "warning" | "info";
}

export interface InquiryFormsView {
  forms: InquiryForm[];
  total: number;
}

export interface InquiryFormEditorView {
  form: InquiryForm;
  revisions: InquiryFormRevision[];
}

export interface InquiriesPageView {
  inquiries: Inquiry[];
  total: number;
  forms: InquiryForm[];
}

export interface InquiryDetailView {
  inquiry: Inquiry;
  revision: InquiryFormRevision | null;
  files: InquiryFile[];
}

export interface NotificationsPageView {
  channels: NotificationChannel[];
  events: NotificationChannelEvent[];
}

export const ORDER_STATUS_LABELS: Record<OrderStatusView, string> = {
  pending: "Pending",
  confirmed: "Confirmed",
  processing: "Processing",
  fulfilled: "Fulfilled",
  completed: "Completed",
  canceled: "Canceled",
};

export const PAYMENT_STATUS_LABELS: Record<PaymentStatusView, string> = {
  awaiting: "Awaiting",
  authorized: "Authorized",
  captured: "Paid",
  partially_refunded: "Partial refund",
  refunded: "Refunded",
  failed: "Failed",
};

export const FULFILLMENT_STATUS_LABELS: Record<FulfillmentStatusView, string> = {
  not_fulfilled: "Unfulfilled",
  partially_fulfilled: "Partial",
  fulfilled: "Fulfilled",
  returned: "Returned",
};

export const PRODUCT_SWATCHES: Array<[string, string]> = [
  ["#A9BFAE", "#425D4A"],
  ["#D9CBB2", "#8A7351"],
  ["#CBA48F", "#7D4B34"],
  ["#D9B274", "#8A5A24"],
  ["#B3C6D1", "#4D6879"],
  ["#B6C1A2", "#5D6B46"],
  ["#D6D2C6", "#8C877A"],
  ["#C4A481", "#6E4A2C"],
];

export function swatchFor(seed: string): [string, string] {
  let hash = 0;
  for (const char of seed) hash = (hash * 31 + char.charCodeAt(0)) % 997;
  return PRODUCT_SWATCHES[hash % PRODUCT_SWATCHES.length] ?? PRODUCT_SWATCHES[0]!;
}

/** Converts API minor units to major units for display. */
export function toMajor(amountMinor: number): number {
  return amountMinor / 100;
}

export function todayIso(): string {
  return new Date().toISOString();
}
