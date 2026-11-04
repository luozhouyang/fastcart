export type OrderStatus =
  "pending" | "confirmed" | "processing" | "fulfilled" | "completed" | "canceled";

export type PaymentStatus =
  "awaiting" | "authorized" | "captured" | "partially_refunded" | "refunded" | "failed";

export type FulfillmentStatus = "not_fulfilled" | "partially_fulfilled" | "fulfilled" | "returned";

export interface RevenuePoint {
  date: string;
  value: number;
}

export interface MockOrderItem {
  title: string;
  variant: string;
  sku: string;
  quantity: number;
  price: number;
}

export interface MockOrder {
  id: string;
  displayNumber: number;
  customer: string;
  email: string;
  placedAt: string;
  status: OrderStatus;
  paymentStatus: PaymentStatus;
  fulfillmentStatus: FulfillmentStatus;
  total: number;
  shipping: number;
  tax: number;
  address: string;
  items: MockOrderItem[];
}

export interface MockProduct {
  id: string;
  title: string;
  handle: string;
  collection: string;
  price: number;
  stock: number;
  status: "active" | "draft" | "archived";
  sold: number;
  revenue: number;
  updatedAt: string;
  swatch: [string, string];
}

export interface MockCustomer {
  id: string;
  name: string;
  email: string;
  status: "active" | "new" | "guest";
  orders: number;
  spent: number;
  lastOrder: string;
  joined: string;
}

export interface MockInventoryRow {
  id: string;
  product: string;
  sku: string;
  location: string;
  stocked: number;
  reserved: number;
  incoming: number;
}

export interface MockLocation {
  id: string;
  name: string;
  address: string;
  units: number;
  items: number;
  primary: boolean;
}

const DAY = 86_400_000;
const ANCHOR = new Date("2026-11-04T12:00:00Z").getTime();

function buildSeries(values: number[]): RevenuePoint[] {
  return values.map((value, index) => {
    const daysAgo = values.length - 1 - index;
    const date = new Date(ANCHOR - daysAgo * DAY);
    return {
      date: date.toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" }),
      value,
    };
  });
}

const DAILY_30 = [
  3180, 2920, 3460, 3890, 3620, 4150, 3880, 4460, 4100, 4720, 4350, 4980, 4640, 5190, 4930, 5440,
  5100, 5720, 5380, 5980, 5610, 6240, 5860, 6480, 6100, 6790, 6360, 7010, 6580, 7260,
];

const DAILY_90 = Array.from({ length: 90 }, (_, index) => {
  const trend = 2180 + index * 34;
  const weekly = [0, 120, 260, 390, 310, 180, -120][index % 7] ?? 0;
  const wave = Math.sin(index / 4.2) * 190;
  return Math.round(trend + weekly + wave);
});

export const revenueSeries: Record<"7d" | "30d" | "90d", RevenuePoint[]> = {
  "7d": buildSeries(DAILY_30.slice(-7)),
  "30d": buildSeries(DAILY_30),
  "90d": buildSeries(DAILY_90),
};

export function seriesTotal(points: RevenuePoint[]): number {
  return points.reduce((total, point) => total + point.value, 0);
}

export const netRevenue = seriesTotal(revenueSeries["30d"]);

export const metrics = [
  {
    key: "revenue",
    label: "Net revenue",
    value: netRevenue,
    format: "money" as const,
    delta: 12.4,
    comparison: "vs previous 30 days",
  },
  {
    key: "orders",
    label: "Orders",
    value: 1284,
    format: "number" as const,
    delta: 8.2,
    comparison: "vs previous 30 days",
  },
  {
    key: "aov",
    label: "Average order value",
    value: 120.23,
    format: "money" as const,
    delta: 3.9,
    comparison: "vs previous 30 days",
  },
  {
    key: "returning",
    label: "Returning customers",
    value: 38.6,
    format: "percent" as const,
    delta: -1.2,
    comparison: "vs previous 30 days",
  },
];

export interface OrderStatusSlice {
  label: string;
  value: number;
  color: string;
}

export const orderStatusBreakdown: OrderStatusSlice[] = [
  { label: "Completed", value: 512, color: "var(--chart-1)" },
  { label: "Fulfilled", value: 348, color: "var(--chart-2)" },
  { label: "Processing", value: 214, color: "var(--chart-3)" },
  { label: "Pending", value: 164, color: "var(--chart-5)" },
  { label: "Canceled", value: 46, color: "var(--chart-4)" },
];

export const totalOrders = orderStatusBreakdown.reduce((total, slice) => total + slice.value, 0);

export const topProducts = [
  {
    id: "p-04",
    title: "Arc Table Lamp",
    collection: "Lighting",
    sold: 26,
    revenue: 4820,
    share: 100,
    swatch: ["#d9b274", "#8a5a24"] as [string, string],
  },
  {
    id: "p-01",
    title: "Ridge Ceramic Carafe",
    collection: "Tableware",
    sold: 68,
    revenue: 3944,
    share: 82,
    swatch: ["#a9bfae", "#425d4a"] as [string, string],
  },
  {
    id: "p-02",
    title: "Loom Linen Throw",
    collection: "Textiles",
    sold: 33,
    revenue: 3168,
    share: 66,
    swatch: ["#d9cbb2", "#8a7351"] as [string, string],
  },
  {
    id: "p-03",
    title: "Field Notes Tumbler",
    collection: "Drinkware",
    sold: 62,
    revenue: 2108,
    share: 44,
    swatch: ["#cba48f", "#7d4b34"] as [string, string],
  },
];

const itemSets: Record<number, MockOrderItem[]> = {
  1042: [
    {
      title: "Arc Table Lamp",
      variant: "Brass / Small",
      sku: "ATL-BRS-04",
      quantity: 1,
      price: 184,
    },
    { title: "Basin Stone Soap", variant: "Cedar", sku: "BSS-CDR-07", quantity: 2, price: 18 },
  ],
  1041: [
    { title: "Ridge Ceramic Carafe", variant: "Sage", sku: "RCC-SAG-01", quantity: 1, price: 58 },
    { title: "Field Notes Tumbler", variant: "Clay", sku: "FNT-CLY-03", quantity: 2, price: 34 },
  ],
  1040: [{ title: "Loom Linen Throw", variant: "Oat", sku: "LLT-OAT-02", quantity: 2, price: 96 }],
  1039: [
    {
      title: "Meridian Serving Board",
      variant: "Walnut",
      sku: "MSB-WAL-08",
      quantity: 1,
      price: 72,
    },
    {
      title: "Northwind Wool Socks",
      variant: "Moss / M",
      sku: "NWS-MOS-06",
      quantity: 3,
      price: 22,
    },
  ],
  1038: [
    { title: "Tide Glass Vessel", variant: "Clear", sku: "TGV-CLR-05", quantity: 1, price: 46 },
  ],
  1037: [
    { title: "Ridge Ceramic Carafe", variant: "Sage", sku: "RCC-SAG-01", quantity: 1, price: 58 },
    { title: "Loom Linen Throw", variant: "Oat", sku: "LLT-OAT-02", quantity: 1, price: 96 },
    { title: "Basin Stone Soap", variant: "Cedar", sku: "BSS-CDR-07", quantity: 1, price: 18 },
  ],
  1036: [
    { title: "Field Notes Tumbler", variant: "Clay", sku: "FNT-CLY-03", quantity: 4, price: 34 },
  ],
  1035: [
    {
      title: "Arc Table Lamp",
      variant: "Brass / Small",
      sku: "ATL-BRS-04",
      quantity: 1,
      price: 184,
    },
    {
      title: "Meridian Serving Board",
      variant: "Walnut",
      sku: "MSB-WAL-08",
      quantity: 1,
      price: 72,
    },
  ],
  1034: [
    {
      title: "Northwind Wool Socks",
      variant: "Moss / L",
      sku: "NWS-MOS-06",
      quantity: 2,
      price: 22,
    },
  ],
  1033: [
    { title: "Loom Linen Throw", variant: "Oat", sku: "LLT-OAT-02", quantity: 1, price: 96 },
    { title: "Basin Stone Soap", variant: "Cedar", sku: "BSS-CDR-07", quantity: 3, price: 18 },
  ],
  1032: [
    { title: "Ridge Ceramic Carafe", variant: "Sage", sku: "RCC-SAG-01", quantity: 2, price: 58 },
  ],
};

export const orders: MockOrder[] = [
  {
    id: "o-1042",
    displayNumber: 1042,
    customer: "Ava Whitfield",
    email: "ava.whitfield@hey.com",
    placedAt: "2026-11-04T09:42:00Z",
    status: "processing",
    paymentStatus: "captured",
    fulfillmentStatus: "not_fulfilled",
    total: 220,
    shipping: 0,
    tax: 0,
    address: "88 Greenpoint Ave, Brooklyn NY 11222",
    items: itemSets[1042] ?? [],
  },
  {
    id: "o-1041",
    displayNumber: 1041,
    customer: "Noah Bennett",
    email: "noah.bennett@gmail.com",
    placedAt: "2026-11-04T08:15:00Z",
    status: "confirmed",
    paymentStatus: "authorized",
    fulfillmentStatus: "not_fulfilled",
    total: 126,
    shipping: 0,
    tax: 0,
    address: "17 Willow St, Portland OR 97204",
    items: itemSets[1041] ?? [],
  },
  {
    id: "o-1040",
    displayNumber: 1040,
    customer: "Mia Alvarez",
    email: "mia.alvarez@icloud.com",
    placedAt: "2026-11-03T18:04:00Z",
    status: "fulfilled",
    paymentStatus: "captured",
    fulfillmentStatus: "fulfilled",
    total: 192,
    shipping: 0,
    tax: 0,
    address: "402 Ashbury St, San Francisco CA 94117",
    items: itemSets[1040] ?? [],
  },
  {
    id: "o-1039",
    displayNumber: 1039,
    customer: "Leo Park",
    email: "leo.park@outlook.com",
    placedAt: "2026-11-03T15:37:00Z",
    status: "pending",
    paymentStatus: "awaiting",
    fulfillmentStatus: "not_fulfilled",
    total: 138,
    shipping: 0,
    tax: 0,
    address: "56 Canal St, New York NY 10002",
    items: itemSets[1039] ?? [],
  },
  {
    id: "o-1038",
    displayNumber: 1038,
    customer: "Isla Fontaine",
    email: "isla@fontaine.studio",
    placedAt: "2026-11-03T12:21:00Z",
    status: "canceled",
    paymentStatus: "refunded",
    fulfillmentStatus: "returned",
    total: 46,
    shipping: 0,
    tax: 0,
    address: "9 Bellevue Rd, Austin TX 78703",
    items: itemSets[1038] ?? [],
  },
  {
    id: "o-1037",
    displayNumber: 1037,
    customer: "Elias Hart",
    email: "elias.hart@gmail.com",
    placedAt: "2026-11-02T17:48:00Z",
    status: "completed",
    paymentStatus: "captured",
    fulfillmentStatus: "fulfilled",
    total: 172,
    shipping: 0,
    tax: 0,
    address: "231 Elm St, Chicago IL 60610",
    items: itemSets[1037] ?? [],
  },
  {
    id: "o-1036",
    displayNumber: 1036,
    customer: "Sofia Marchetti",
    email: "sofia.marchetti@icloud.com",
    placedAt: "2026-11-02T11:09:00Z",
    status: "fulfilled",
    paymentStatus: "captured",
    fulfillmentStatus: "fulfilled",
    total: 136,
    shipping: 0,
    tax: 0,
    address: "12 Via Roma, Brooklyn NY 11201",
    items: itemSets[1036] ?? [],
  },
  {
    id: "o-1035",
    displayNumber: 1035,
    customer: "Owen Caldwell",
    email: "owen.caldwell@hey.com",
    placedAt: "2026-11-01T20:33:00Z",
    status: "processing",
    paymentStatus: "captured",
    fulfillmentStatus: "partially_fulfilled",
    total: 256,
    shipping: 0,
    tax: 0,
    address: "77 Pine St, Seattle WA 98101",
    items: itemSets[1035] ?? [],
  },
  {
    id: "o-1034",
    displayNumber: 1034,
    customer: "Hana Sato",
    email: "hana.sato@gmail.com",
    placedAt: "2026-11-01T13:12:00Z",
    status: "confirmed",
    paymentStatus: "authorized",
    fulfillmentStatus: "not_fulfilled",
    total: 44,
    shipping: 0,
    tax: 0,
    address: "4 Sakura Ave, Los Angeles CA 90012",
    items: itemSets[1034] ?? [],
  },
  {
    id: "o-1033",
    displayNumber: 1033,
    customer: "Felix Moreau",
    email: "felix.moreau@proton.me",
    placedAt: "2026-10-31T16:55:00Z",
    status: "completed",
    paymentStatus: "captured",
    fulfillmentStatus: "fulfilled",
    total: 150,
    shipping: 0,
    tax: 0,
    address: "19 Rue Cler, New Orleans LA 70130",
    items: itemSets[1033] ?? [],
  },
  {
    id: "o-1032",
    displayNumber: 1032,
    customer: "Nina Petrova",
    email: "nina.petrova@icloud.com",
    placedAt: "2026-10-31T10:05:00Z",
    status: "fulfilled",
    paymentStatus: "captured",
    fulfillmentStatus: "fulfilled",
    total: 116,
    shipping: 0,
    tax: 0,
    address: "6 Hollow Ln, Hudson NY 12534",
    items: itemSets[1032] ?? [],
  },
];

export const products: MockProduct[] = [
  {
    id: "p-01",
    title: "Ridge Ceramic Carafe",
    handle: "ridge-ceramic-carafe",
    collection: "Tableware",
    price: 58,
    stock: 42,
    status: "active",
    sold: 68,
    revenue: 3944,
    updatedAt: "2026-11-04T08:12:00Z",
    swatch: ["#a9bfae", "#425d4a"],
  },
  {
    id: "p-02",
    title: "Loom Linen Throw",
    handle: "loom-linen-throw",
    collection: "Textiles",
    price: 96,
    stock: 18,
    status: "active",
    sold: 33,
    revenue: 3168,
    updatedAt: "2026-11-03T15:40:00Z",
    swatch: ["#d9cbb2", "#8a7351"],
  },
  {
    id: "p-03",
    title: "Field Notes Tumbler",
    handle: "field-notes-tumbler",
    collection: "Drinkware",
    price: 34,
    stock: 126,
    status: "active",
    sold: 62,
    revenue: 2108,
    updatedAt: "2026-11-03T09:26:00Z",
    swatch: ["#cba48f", "#7d4b34"],
  },
  {
    id: "p-04",
    title: "Arc Table Lamp",
    handle: "arc-table-lamp",
    collection: "Lighting",
    price: 184,
    stock: 6,
    status: "active",
    sold: 26,
    revenue: 4820,
    updatedAt: "2026-11-02T18:03:00Z",
    swatch: ["#d9b274", "#8a5a24"],
  },
  {
    id: "p-05",
    title: "Tide Glass Vessel",
    handle: "tide-glass-vessel",
    collection: "Decor",
    price: 46,
    stock: 0,
    status: "draft",
    sold: 0,
    revenue: 0,
    updatedAt: "2026-11-02T10:44:00Z",
    swatch: ["#b3c6d1", "#4d6879"],
  },
  {
    id: "p-06",
    title: "Northwind Wool Socks",
    handle: "northwind-wool-socks",
    collection: "Apparel",
    price: 22,
    stock: 64,
    status: "active",
    sold: 94,
    revenue: 2068,
    updatedAt: "2026-11-01T16:20:00Z",
    swatch: ["#b6c1a2", "#5d6b46"],
  },
  {
    id: "p-07",
    title: "Basin Stone Soap",
    handle: "basin-stone-soap",
    collection: "Bath",
    price: 18,
    stock: 210,
    status: "active",
    sold: 138,
    revenue: 2484,
    updatedAt: "2026-10-30T12:58:00Z",
    swatch: ["#d6d2c6", "#8c877a"],
  },
  {
    id: "p-08",
    title: "Meridian Serving Board",
    handle: "meridian-serving-board",
    collection: "Kitchen",
    price: 72,
    stock: 31,
    status: "active",
    sold: 41,
    revenue: 2952,
    updatedAt: "2026-10-29T09:31:00Z",
    swatch: ["#c4a481", "#6e4a2c"],
  },
];

export const customers: MockCustomer[] = [
  {
    id: "c-01",
    name: "Ava Whitfield",
    email: "ava.whitfield@hey.com",
    status: "active",
    orders: 12,
    spent: 1840,
    lastOrder: "2026-11-04T09:42:00Z",
    joined: "2025-03-12T00:00:00Z",
  },
  {
    id: "c-02",
    name: "Noah Bennett",
    email: "noah.bennett@gmail.com",
    status: "active",
    orders: 7,
    spent: 964,
    lastOrder: "2026-11-04T08:15:00Z",
    joined: "2025-08-03T00:00:00Z",
  },
  {
    id: "c-03",
    name: "Mia Alvarez",
    email: "mia.alvarez@icloud.com",
    status: "active",
    orders: 9,
    spent: 1216,
    lastOrder: "2026-11-03T18:04:00Z",
    joined: "2025-01-27T00:00:00Z",
  },
  {
    id: "c-04",
    name: "Leo Park",
    email: "leo.park@outlook.com",
    status: "new",
    orders: 1,
    spent: 138,
    lastOrder: "2026-11-03T15:37:00Z",
    joined: "2026-11-03T00:00:00Z",
  },
  {
    id: "c-05",
    name: "Isla Fontaine",
    email: "isla@fontaine.studio",
    status: "active",
    orders: 4,
    spent: 512,
    lastOrder: "2026-11-03T12:21:00Z",
    joined: "2025-11-19T00:00:00Z",
  },
  {
    id: "c-06",
    name: "Elias Hart",
    email: "elias.hart@gmail.com",
    status: "guest",
    orders: 2,
    spent: 344,
    lastOrder: "2026-11-02T17:48:00Z",
    joined: "2026-10-02T00:00:00Z",
  },
  {
    id: "c-07",
    name: "Sofia Marchetti",
    email: "sofia.marchetti@icloud.com",
    status: "active",
    orders: 15,
    spent: 2284,
    lastOrder: "2026-11-02T11:09:00Z",
    joined: "2024-09-08T00:00:00Z",
  },
  {
    id: "c-08",
    name: "Owen Caldwell",
    email: "owen.caldwell@hey.com",
    status: "active",
    orders: 6,
    spent: 806,
    lastOrder: "2026-11-01T20:33:00Z",
    joined: "2025-06-21T00:00:00Z",
  },
  {
    id: "c-09",
    name: "Hana Sato",
    email: "hana.sato@gmail.com",
    status: "new",
    orders: 1,
    spent: 44,
    lastOrder: "2026-11-01T13:12:00Z",
    joined: "2026-11-01T00:00:00Z",
  },
  {
    id: "c-10",
    name: "Felix Moreau",
    email: "felix.moreau@proton.me",
    status: "active",
    orders: 11,
    spent: 1476,
    lastOrder: "2026-10-31T16:55:00Z",
    joined: "2025-02-14T00:00:00Z",
  },
];

export const inventoryRows: MockInventoryRow[] = [
  {
    id: "i-01",
    product: "Ridge Ceramic Carafe",
    sku: "RCC-SAG-01",
    location: "Brooklyn Studio",
    stocked: 42,
    reserved: 6,
    incoming: 0,
  },
  {
    id: "i-02",
    product: "Loom Linen Throw",
    sku: "LLT-OAT-02",
    location: "Brooklyn Studio",
    stocked: 18,
    reserved: 4,
    incoming: 24,
  },
  {
    id: "i-03",
    product: "Field Notes Tumbler",
    sku: "FNT-CLY-03",
    location: "Portland Annex",
    stocked: 126,
    reserved: 12,
    incoming: 0,
  },
  {
    id: "i-04",
    product: "Arc Table Lamp",
    sku: "ATL-BRS-04",
    location: "Brooklyn Studio",
    stocked: 6,
    reserved: 3,
    incoming: 12,
  },
  {
    id: "i-05",
    product: "Tide Glass Vessel",
    sku: "TGV-CLR-05",
    location: "Portland Annex",
    stocked: 0,
    reserved: 0,
    incoming: 0,
  },
  {
    id: "i-06",
    product: "Northwind Wool Socks",
    sku: "NWS-MOS-06",
    location: "Portland Annex",
    stocked: 64,
    reserved: 8,
    incoming: 0,
  },
  {
    id: "i-07",
    product: "Basin Stone Soap",
    sku: "BSS-CDR-07",
    location: "Brooklyn Studio",
    stocked: 210,
    reserved: 18,
    incoming: 0,
  },
  {
    id: "i-08",
    product: "Meridian Serving Board",
    sku: "MSB-WAL-08",
    location: "Brooklyn Studio",
    stocked: 31,
    reserved: 5,
    incoming: 0,
  },
];

export const locations: MockLocation[] = [
  {
    id: "l-01",
    name: "Brooklyn Studio",
    address: "214 Wythe Ave, Brooklyn NY",
    units: 507,
    items: 6,
    primary: true,
  },
  {
    id: "l-02",
    name: "Portland Annex",
    address: "88 SE Ash St, Portland OR",
    units: 190,
    items: 3,
    primary: false,
  },
  {
    id: "l-03",
    name: "Transit / Inbound",
    address: "Awaiting carrier receipt",
    units: 36,
    items: 2,
    primary: false,
  },
];

export const notifications = [
  {
    id: "n-1",
    title: "New order #1042",
    body: "Ava Whitfield placed a $220.00 order.",
    time: "9 min ago",
    tone: "brand" as const,
  },
  {
    id: "n-2",
    title: "Low stock: Arc Table Lamp",
    body: "Only 6 units left at Brooklyn Studio.",
    time: "42 min ago",
    tone: "warning" as const,
  },
  {
    id: "n-3",
    title: "Payout scheduled",
    body: "$18,420.00 arrives on 6 November.",
    time: "2 hours ago",
    tone: "info" as const,
  },
];
