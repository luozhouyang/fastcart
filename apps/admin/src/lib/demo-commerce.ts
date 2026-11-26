import type {
  GiftCard,
  PaymentRefund,
  Price,
  PriceList,
  Promotion,
  WebhookDelivery,
  WebhookEndpoint,
} from "@fastcart/shared-types";

import type { Campaign } from "@/lib/campaigns";
import type { AdminPayment, AdminPaymentSession } from "@/lib/payments";

/** Fixed demo clock so seeded timestamps stay stable between renders. */
const DEMO_CLOCK = Math.floor(new Date("2026-11-26T12:00:00Z").getTime() / 1000);
const DAY = 86_400;

function daysAgo(days: number): number {
  return DEMO_CLOCK - days * DAY;
}

/** Sample price lists rendered by the demo. */
export function demoPriceLists(): PriceList[] {
  return [
    {
      id: "pl_demo_wholesale",
      title: "Wholesale spring",
      description: "Trade accounts ordering at least six units.",
      price_type: "override",
      status: "active",
      starts_at: daysAgo(30),
      ends_at: DEMO_CLOCK + 45 * DAY,
      metadata: {},
      rules: [],
      created_at: daysAgo(32),
      updated_at: daysAgo(5),
    },
    {
      id: "pl_demo_sample",
      title: "Studio sample sale",
      description: "Display units with light shelf wear.",
      price_type: "sale",
      status: "active",
      starts_at: daysAgo(10),
      ends_at: DEMO_CLOCK + 10 * DAY,
      metadata: {},
      rules: [],
      created_at: daysAgo(12),
      updated_at: daysAgo(2),
    },
    {
      id: "pl_demo_archive",
      title: "Archive clearance",
      description: null,
      price_type: "override",
      status: "draft",
      starts_at: null,
      ends_at: null,
      metadata: {},
      rules: [],
      created_at: daysAgo(60),
      updated_at: daysAgo(60),
    },
  ];
}

/** Price rows grouped by demo price list id. */
export function demoPriceListPrices(): Record<string, Price[]> {
  return {
    pl_demo_wholesale: [
      {
        id: "price_demo_wholesale_1",
        price_list_id: "pl_demo_wholesale",
        variant_id: "var_ridge_sage",
        currency_code: "USD",
        amount: 12_800,
        min_quantity: 6,
        max_quantity: null,
        customer_group_id: null,
        sales_channel_id: null,
        starts_at: null,
        ends_at: null,
        metadata: {},
        created_at: daysAgo(30),
        updated_at: daysAgo(30),
      },
      {
        id: "price_demo_wholesale_2",
        price_list_id: "pl_demo_wholesale",
        variant_id: "var_pond_stone",
        currency_code: "USD",
        amount: 9_600,
        min_quantity: 6,
        max_quantity: null,
        customer_group_id: null,
        sales_channel_id: null,
        starts_at: null,
        ends_at: null,
        metadata: {},
        created_at: daysAgo(30),
        updated_at: daysAgo(30),
      },
    ],
    pl_demo_sample: [
      {
        id: "price_demo_sample_1",
        price_list_id: "pl_demo_sample",
        variant_id: "var_ridge_sage",
        currency_code: "USD",
        amount: 14_900,
        min_quantity: 1,
        max_quantity: 2,
        customer_group_id: null,
        sales_channel_id: null,
        starts_at: null,
        ends_at: null,
        metadata: {},
        created_at: daysAgo(10),
        updated_at: daysAgo(10),
      },
    ],
    pl_demo_archive: [],
  };
}

/** Sample campaigns shown by the demo. */
export function demoCampaigns(): Campaign[] {
  return [
    {
      id: "camp_demo_autumn",
      name: "Autumn studio edit",
      description: "Seasonal promotion bundle for the studio collection.",
      budget_type: "spend",
      currency_code: "USD",
      budget_limit: 250_000,
      budget_used: 78_500,
      starts_at: daysAgo(14),
      ends_at: DEMO_CLOCK + 12 * DAY,
      metadata: {},
      created_at: daysAgo(16),
      updated_at: daysAgo(1),
    },
    {
      id: "camp_demo_welcome",
      name: "Welcome series",
      description: null,
      budget_type: "usage",
      currency_code: null,
      budget_limit: 500,
      budget_used: 137,
      starts_at: null,
      ends_at: null,
      metadata: {},
      created_at: daysAgo(90),
      updated_at: daysAgo(3),
    },
    {
      id: "camp_demo_archive",
      name: "Archive clearance",
      description: "Retired colourways.",
      budget_type: "usage",
      currency_code: null,
      budget_limit: null,
      budget_used: 0,
      starts_at: null,
      ends_at: daysAgo(20),
      metadata: {},
      created_at: daysAgo(120),
      updated_at: daysAgo(20),
    },
  ];
}

/** Promotions that the demo campaign link picker can offer. */
export function demoCampaignPromotionChoices(): Array<Pick<Promotion, "id" | "title">> {
  return [
    { id: "promo_demo_summer", title: "Summer studio edit" },
    { id: "promo_demo_ship", title: "Free shipping over $75" },
    { id: "promo_demo_welcome", title: "Welcome offer" },
    { id: "promo_demo_vip", title: "VIP appreciation" },
  ];
}

/** Linked promotion ids grouped by demo campaign id. */
export function demoCampaignLinks(): Record<string, string[]> {
  return {
    camp_demo_autumn: ["promo_demo_summer", "promo_demo_ship"],
    camp_demo_welcome: ["promo_demo_welcome"],
    camp_demo_archive: [],
  };
}

/** Sample webhook endpoints shown by the demo. */
export function demoWebhookEndpoints(): WebhookEndpoint[] {
  return [
    {
      id: "wh_demo_orders",
      url: "https://hooks.halcyonsupply.test/fastcart/orders",
      events: ["order.placed", "order.canceled", "payment.captured"],
      is_active: true,
      metadata: {},
      created_at: daysAgo(75),
      updated_at: daysAgo(4),
    },
    {
      id: "wh_demo_inventory",
      url: "https://ops.halcyonsupply.test/inventory-sync",
      events: ["inventory.updated"],
      is_active: false,
      metadata: {},
      created_at: daysAgo(40),
      updated_at: daysAgo(18),
    },
  ];
}

/** Sample deliveries for a demo endpoint. */
export function demoWebhookDeliveries(endpointId: string): WebhookDelivery[] {
  if (endpointId !== "wh_demo_orders") return [];
  return [
    {
      id: "whd_demo_1",
      endpoint_id: endpointId,
      event_id: "evt_demo_1042",
      event_type: "order.placed",
      payload: { order_id: "order_1042", total: 22_000, currency: "USD" },
      status: "delivered",
      attempts: 1,
      response_status: 200,
      response_body: "ok",
      next_retry_at: null,
      created_at: daysAgo(1),
      updated_at: daysAgo(1),
    },
    {
      id: "whd_demo_2",
      endpoint_id: endpointId,
      event_id: "evt_demo_1041",
      event_type: "payment.captured",
      payload: { payment_id: "pay_demo_1041", amount: 12_400 },
      status: "failed",
      attempts: 4,
      response_status: 503,
      response_body: "upstream unavailable",
      next_retry_at: DEMO_CLOCK + 1_800,
      created_at: daysAgo(2),
      updated_at: daysAgo(1),
    },
  ];
}

/** Sample payments shown by the demo. */
export function demoPayments(): AdminPayment[] {
  return [
    {
      id: "pay_demo_1042",
      order_id: "order_1042",
      provider: "stripe",
      provider_payment_id: "pi_demo_1042",
      amount: 22_000,
      currency_code: "USD",
      status: "captured",
      captured_at: daysAgo(1),
      metadata: { session_id: "paysess_demo_1042" },
      created_at: daysAgo(1),
      updated_at: daysAgo(1),
    },
    {
      id: "pay_demo_1041",
      order_id: "order_1041",
      provider: "stripe",
      provider_payment_id: "pi_demo_1041",
      amount: 12_400,
      currency_code: "USD",
      status: "partially_refunded",
      captured_at: daysAgo(3),
      metadata: { session_id: "paysess_demo_1041" },
      created_at: daysAgo(3),
      updated_at: daysAgo(1),
    },
    {
      id: "pay_demo_1039",
      order_id: "order_1039",
      provider: "manual",
      provider_payment_id: "manual_demo_1039",
      amount: 8_000,
      currency_code: "USD",
      status: "refunded",
      captured_at: daysAgo(9),
      metadata: {},
      created_at: daysAgo(9),
      updated_at: daysAgo(6),
    },
    {
      id: "pay_demo_1038",
      order_id: "order_1038",
      provider: "stripe",
      provider_payment_id: null,
      amount: 5_200,
      currency_code: "USD",
      status: "failed",
      captured_at: null,
      metadata: {},
      created_at: daysAgo(11),
      updated_at: daysAgo(11),
    },
  ];
}

/** Sample refunds for a demo payment. */
export function demoPaymentRefunds(paymentId: string): PaymentRefund[] {
  if (paymentId !== "pay_demo_1041") {
    if (paymentId === "pay_demo_1039") {
      return [
        {
          id: "refund_demo_1",
          payment_id: paymentId,
          order_id: "order_1039",
          amount: 8_000,
          currency_code: "USD",
          status: "succeeded",
          provider_refund_id: "manual_refund_demo_1",
          reason: "Customer changed their mind",
          created_at: daysAgo(6),
          updated_at: daysAgo(6),
        },
      ];
    }
    return [];
  }
  return [
    {
      id: "refund_demo_2",
      payment_id: paymentId,
      order_id: "order_1041",
      amount: 2_400,
      currency_code: "USD",
      status: "succeeded",
      provider_refund_id: "re_demo_2",
      reason: "Chipped lid",
      created_at: daysAgo(1),
      updated_at: daysAgo(1),
    },
  ];
}

/** Sample pending payment session used by the demo session actions. */
export function demoPaymentSession(sessionId: string): AdminPaymentSession {
  return {
    id: sessionId,
    cart_id: null,
    checkout_id: "checkout_demo_1",
    order_id: null,
    provider: "stripe",
    provider_session_id: "pi_demo_pending",
    amount: 14_800,
    currency_code: "USD",
    status: "requires_action",
    data: {},
    created_at: daysAgo(0),
    updated_at: daysAgo(0),
  };
}

/** Sample gift cards shown by the demo. */
export function demoGiftCards(): GiftCard[] {
  return [
    {
      id: "gift_demo_7001",
      code: "7QK2M9XR4TZB1HVN",
      currency_code: "USD",
      initial_amount_minor: 10_000,
      balance_minor: 7_500,
      status: "active",
      metadata: {},
      created_at: daysAgo(12),
      updated_at: daysAgo(2),
    },
    {
      id: "gift_demo_7002",
      code: "STUDIO2026DEMO01",
      currency_code: "USD",
      initial_amount_minor: 5_000,
      balance_minor: 0,
      status: "active",
      metadata: {},
      created_at: daysAgo(30),
      updated_at: daysAgo(7),
    },
    {
      id: "gift_demo_7003",
      code: "HALCYON-FRIENDS",
      currency_code: "USD",
      initial_amount_minor: 2_500,
      balance_minor: 2_500,
      status: "disabled",
      metadata: {},
      created_at: daysAgo(45),
      updated_at: daysAgo(20),
    },
  ];
}
