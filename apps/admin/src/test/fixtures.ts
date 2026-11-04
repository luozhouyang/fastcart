import type {
  Collection,
  Customer,
  Domain,
  InventoryItem,
  InventoryLevel,
  InventoryLocation,
  Order,
  OrderItem,
  OrderStatusHistory,
  PageResult,
  Payment,
  Price,
  Product,
  Refund,
  Store,
  Variant,
} from "@fastcart/shared-types";

const NOW = 1_793_760_000;

export function page<T>(data: T[], total = data.length): PageResult<T> {
  return {
    data,
    pagination: { limit: Math.max(data.length, 1), offset: 0, total },
  };
}

export function variant(overrides: Partial<Variant> = {}): Variant {
  return {
    id: "variant-1",
    product_id: "product-1",
    title: "Default",
    sku: "SKU-1",
    barcode: null,
    status: "active",
    weight: null,
    dimensions: { length: null, height: null, width: null },
    manage_inventory: true,
    allow_backorder: false,
    tax_category_id: null,
    metadata: {},
    option_value_ids: [],
    created_at: NOW,
    updated_at: NOW,
    deleted_at: null,
    ...overrides,
  };
}

export function collection(overrides: Partial<Collection> = {}): Collection {
  return {
    id: "collection-1",
    title: "Tableware",
    handle: "tableware",
    description: null,
    is_published: true,
    rank: 0,
    metadata: {},
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function product(overrides: Partial<Product> = {}): Product {
  return {
    id: "product-1",
    title: "Ridge Ceramic Carafe",
    subtitle: null,
    description: null,
    handle: "ridge-ceramic-carafe",
    status: "active",
    thumbnail: null,
    weight: null,
    dimensions: { length: null, height: null, width: null },
    metadata: {},
    created_at: NOW,
    updated_at: NOW,
    variants: [variant()],
    options: [],
    images: [],
    categories: [],
    collections: [collection()],
    tags: [],
    ...overrides,
  };
}

export function orderItem(overrides: Partial<OrderItem> = {}): OrderItem {
  return {
    id: "item-1",
    order_id: "order-1",
    product_id: "product-1",
    variant_id: "variant-1",
    product_title: "Ridge Ceramic Carafe",
    variant_title: "Sage",
    sku: "RCC-SAG-01",
    thumbnail: null,
    unit_price: 18400,
    quantity: 1,
    subtotal: 18400,
    discount_total: 0,
    tax_total: 0,
    total: 18400,
    is_tax_inclusive: false,
    requires_shipping: true,
    metadata: {},
    adjustments: [],
    ...overrides,
  };
}

export function statusHistory(overrides: Partial<OrderStatusHistory> = {}): OrderStatusHistory {
  return {
    id: "history-1",
    order_id: "order-1",
    status: "pending",
    previous_status: null,
    note: null,
    created_at: NOW,
    ...overrides,
  };
}

export function refund(overrides: Partial<Refund> = {}): Refund {
  return {
    id: "refund-1",
    order_id: "order-1",
    payment_id: "payment-1",
    amount: 5000,
    currency_code: "USD",
    status: "succeeded",
    reason: "Returned damaged",
    provider_refund_id: null,
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function order(overrides: Partial<Order> = {}): Order {
  return {
    id: "order-1",
    display_number: 1042,
    customer_id: "customer-1",
    email: "ava.whitfield@hey.com",
    currency_code: "USD",
    status: "processing",
    payment_status: "captured",
    fulfillment_status: "not_fulfilled",
    refund_status: "not_refunded",
    subtotal: 22000,
    discount_total: 0,
    shipping_total: 0,
    tax_total: 0,
    total: 22000,
    shipping_address: {
      address_1: "88 Greenpoint Ave",
      city: "Brooklyn",
      province: "NY",
      postal_code: "11222",
      country_code: "US",
    },
    billing_address: null,
    metadata: { customer_note: "Leave with the concierge." },
    created_at: NOW,
    updated_at: NOW,
    canceled_at: null,
    items: [orderItem()],
    adjustments: [],
    status_history: [statusHistory()],
    fulfillments: [],
    refunds: [],
    ...overrides,
  };
}

export function customer(overrides: Partial<Customer> = {}): Customer {
  return {
    id: "customer-1",
    email: "ava.whitfield@hey.com",
    first_name: "Ava",
    last_name: "Whitfield",
    phone: null,
    has_account: true,
    status: "active",
    email_verified_at: null,
    metadata: {},
    created_at: NOW - 86_400_000 * 90,
    updated_at: NOW,
    deleted_at: null,
    ...overrides,
  };
}

export function price(overrides: Partial<Price> = {}): Price {
  return {
    id: "price-1",
    price_list_id: null,
    variant_id: "variant-1",
    currency_code: "USD",
    amount: 5800,
    min_quantity: 1,
    max_quantity: null,
    customer_group_id: null,
    sales_channel_id: null,
    starts_at: null,
    ends_at: null,
    metadata: {},
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function location(overrides: Partial<InventoryLocation> = {}): InventoryLocation {
  return {
    id: "location-1",
    name: "Brooklyn Studio",
    address: { address_1: "214 Wythe Ave", city: "Brooklyn", province: "NY" },
    metadata: {},
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function inventoryItem(overrides: Partial<InventoryItem> = {}): InventoryItem {
  return {
    id: "inventory-item-1",
    sku: "RCC-SAG-01",
    variant_id: "variant-1",
    metadata: {},
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function inventoryLevel(overrides: Partial<InventoryLevel> = {}): InventoryLevel {
  return {
    id: "level-1",
    inventory_item_id: "inventory-item-1",
    location_id: "location-1",
    stocked_quantity: 42,
    reserved_quantity: 6,
    incoming_quantity: 0,
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function payment(overrides: Partial<Payment> = {}): Payment {
  return {
    id: "payment-1",
    order_id: "order-1",
    provider: "manual",
    provider_payment_id: null,
    amount: 22000,
    currency_code: "USD",
    status: "captured",
    captured_at: NOW,
    metadata: {},
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function store(overrides: Partial<Store> = {}): Store {
  return {
    id: "store-1",
    name: "Halcyon Supply",
    slug: "halcyon-supply",
    status: "active",
    primary_domain: "halcyonsupply.test",
    locale: "en-US",
    currency_code: "USD",
    timezone: "UTC",
    created_at: NOW,
    updated_at: NOW,
    ...overrides,
  };
}

export function domain(overrides: Partial<Domain> = {}): Domain {
  return {
    id: "domain-1",
    store_id: "store-1",
    hostname: "halcyonsupply.test",
    is_primary: true,
    verified_at: NOW,
    ...overrides,
  };
}
