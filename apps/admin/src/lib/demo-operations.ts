import type { PublishableApiKey } from "@fastcart/admin-sdk";
import type {
  Region,
  SalesChannel,
  ShippingProfile,
  ShippingRate,
  TaxCategory,
  TaxRate,
  TaxRegion,
} from "@fastcart/shared-types";

import type { ShippingOptionRecord } from "@/lib/operations";

/** Fixed demo clock so seeded timestamps stay stable between renders. */
const DEMO_CLOCK = Math.floor(new Date("2026-11-08T12:00:00Z").getTime() / 1000);
const DAY = 86_400;

function daysAgo(days: number): number {
  return DEMO_CLOCK - days * DAY;
}

/** Sample store regions used by the demo dataset. */
export function demoRegions(): Region[] {
  return [
    {
      id: "reg_us",
      name: "United States",
      currency_code: "USD",
      countries: ["US"],
      tax_region_id: "txreg_us",
      payment_providers: ["manual", "stripe"],
      is_default: true,
      metadata: {},
      created_at: daysAgo(120),
      updated_at: daysAgo(3),
    },
    {
      id: "reg_ca",
      name: "Canada",
      currency_code: "CAD",
      countries: ["CA"],
      tax_region_id: "txreg_ca",
      payment_providers: ["manual"],
      is_default: false,
      metadata: {},
      created_at: daysAgo(90),
      updated_at: daysAgo(12),
    },
    {
      id: "reg_eu",
      name: "European Union",
      currency_code: "EUR",
      countries: ["DE", "FR", "NL", "BE"],
      tax_region_id: "txreg_de",
      payment_providers: ["stripe"],
      is_default: false,
      metadata: {},
      created_at: daysAgo(60),
      updated_at: daysAgo(8),
    },
  ];
}

/** Sample publishable keys; only prefixes are ever listed. */
export function demoPublishableKeys(): PublishableApiKey[] {
  return [
    {
      id: "pk_demo_live",
      title: "Storefront (production)",
      token_prefix: "pk_live_ab12",
      sales_channel_ids: ["sc_default"],
      revoked_at: null,
      last_used_at: daysAgo(0),
      created_at: daysAgo(120),
    },
    {
      id: "pk_demo_preview",
      title: "Preview builds",
      token_prefix: "pk_test_cd34",
      sales_channel_ids: ["sc_default"],
      revoked_at: daysAgo(14),
      last_used_at: daysAgo(20),
      created_at: daysAgo(75),
    },
  ];
}

/** Sample sales channels available when creating a key. */
export function demoSalesChannels(): SalesChannel[] {
  return [
    {
      id: "sc_default",
      name: "Default",
      description: "Default sales channel",
      is_disabled: false,
      metadata: {},
      created_at: daysAgo(120),
      updated_at: daysAgo(120),
    },
    {
      id: "sc_wholesale",
      name: "Wholesale",
      description: "B2B ordering",
      is_disabled: false,
      metadata: {},
      created_at: daysAgo(45),
      updated_at: daysAgo(45),
    },
  ];
}

/** Sample shipping profiles. */
export function demoShippingProfiles(): ShippingProfile[] {
  return [
    {
      id: "sp_default",
      name: "Default",
      profile_type: "default",
      metadata: {},
      created_at: daysAgo(120),
      updated_at: daysAgo(120),
    },
    {
      id: "sp_oversized",
      name: "Oversized items",
      profile_type: "default",
      metadata: {},
      created_at: daysAgo(40),
      updated_at: daysAgo(40),
    },
  ];
}

/** Sample flat-rate shipping options. */
export function demoShippingOptions(): ShippingOptionRecord[] {
  return [
    {
      id: "so_standard",
      profile_id: "sp_default",
      name: "Standard shipping",
      price_type: "flat",
      amount_minor: 500,
      currency: "USD",
      provider_id: "manual",
      is_enabled: true,
      metadata: {},
      created_at: daysAgo(120),
      updated_at: daysAgo(6),
    },
    {
      id: "so_express",
      profile_id: "sp_default",
      name: "Express shipping",
      price_type: "flat",
      amount_minor: 1_800,
      currency: "USD",
      provider_id: "manual",
      is_enabled: true,
      metadata: {},
      created_at: daysAgo(110),
      updated_at: daysAgo(9),
    },
    {
      id: "so_pickup",
      profile_id: "sp_default",
      name: "Store pickup",
      price_type: "flat",
      amount_minor: 0,
      currency: "USD",
      provider_id: "manual",
      is_enabled: true,
      metadata: {},
      created_at: daysAgo(80),
      updated_at: daysAgo(80),
    },
    {
      id: "so_freight",
      profile_id: "sp_oversized",
      name: "Freight",
      price_type: "flat",
      amount_minor: 4_500,
      currency: "USD",
      provider_id: "manual",
      is_enabled: false,
      metadata: {},
      created_at: daysAgo(38),
      updated_at: daysAgo(11),
    },
  ];
}

/** Sample destination rates keyed by shipping option id. */
export function demoShippingRates(): Record<string, ShippingRate[]> {
  return {
    so_standard: [
      {
        id: "sr_standard_us",
        shipping_option_id: "so_standard",
        country_code: "US",
        province_code: null,
        amount_minor: 500,
        currency: "USD",
        min_subtotal: null,
        max_subtotal: null,
        metadata: {},
        created_at: daysAgo(120),
      },
      {
        id: "sr_standard_ca",
        shipping_option_id: "so_standard",
        country_code: "CA",
        province_code: null,
        amount_minor: 900,
        currency: "USD",
        min_subtotal: null,
        max_subtotal: null,
        metadata: {},
        created_at: daysAgo(90),
      },
      {
        id: "sr_standard_rest",
        shipping_option_id: "so_standard",
        country_code: null,
        province_code: null,
        amount_minor: 1_400,
        currency: "USD",
        min_subtotal: 7_500,
        max_subtotal: null,
        metadata: {},
        created_at: daysAgo(60),
      },
    ],
    so_express: [
      {
        id: "sr_express_us",
        shipping_option_id: "so_express",
        country_code: "US",
        province_code: null,
        amount_minor: 1_800,
        currency: "USD",
        min_subtotal: null,
        max_subtotal: null,
        metadata: {},
        created_at: daysAgo(110),
      },
      {
        id: "sr_express_us_ca",
        shipping_option_id: "so_express",
        country_code: "US",
        province_code: "CA",
        amount_minor: 2_500,
        currency: "USD",
        min_subtotal: 5_000,
        max_subtotal: null,
        metadata: {},
        created_at: daysAgo(100),
      },
    ],
    so_pickup: [
      {
        id: "sr_pickup_us",
        shipping_option_id: "so_pickup",
        country_code: "US",
        province_code: null,
        amount_minor: 0,
        currency: "USD",
        min_subtotal: null,
        max_subtotal: null,
        metadata: {},
        created_at: daysAgo(80),
      },
    ],
    so_freight: [
      {
        id: "sr_freight_us",
        shipping_option_id: "so_freight",
        country_code: "US",
        province_code: null,
        amount_minor: 4_500,
        currency: "USD",
        min_subtotal: 0,
        max_subtotal: 100_000,
        metadata: {},
        created_at: daysAgo(38),
      },
    ],
  };
}

/** Sample tax categories. */
export function demoTaxCategories(): TaxCategory[] {
  return [
    {
      id: "txcat_standard",
      name: "Standard",
      code: "standard",
      description: "Default rate for most physical goods",
      created_at: daysAgo(120),
      updated_at: daysAgo(120),
    },
    {
      id: "txcat_reduced",
      name: "Reduced",
      code: "reduced",
      description: "Books, food and other reduced-rate goods",
      created_at: daysAgo(115),
      updated_at: daysAgo(115),
    },
    {
      id: "txcat_zero",
      name: "Zero-rated",
      code: "zero",
      description: "Exempt from sales tax",
      created_at: daysAgo(100),
      updated_at: daysAgo(100),
    },
  ];
}

/** Sample tax regions. */
export function demoTaxRegions(): TaxRegion[] {
  return [
    {
      id: "txreg_us",
      name: "United States",
      country_code: "US",
      province_code: null,
      metadata: {},
      created_at: daysAgo(120),
      updated_at: daysAgo(120),
    },
    {
      id: "txreg_ca",
      name: "Canada",
      country_code: "CA",
      province_code: null,
      metadata: {},
      created_at: daysAgo(90),
      updated_at: daysAgo(90),
    },
    {
      id: "txreg_de",
      name: "Germany",
      country_code: "DE",
      province_code: null,
      metadata: {},
      created_at: daysAgo(60),
      updated_at: daysAgo(60),
    },
  ];
}

/** Sample tax rates. */
export function demoTaxRates(): TaxRate[] {
  return [
    {
      id: "txrate_us_standard",
      tax_region_id: "txreg_us",
      tax_category_id: "txcat_standard",
      name: "US sales tax",
      rate_bps: 825,
      is_inclusive: false,
      is_default: true,
      metadata: {},
      created_at: daysAgo(120),
      updated_at: daysAgo(3),
    },
    {
      id: "txrate_us_reduced",
      tax_region_id: "txreg_us",
      tax_category_id: "txcat_reduced",
      name: "US reduced rate",
      rate_bps: 400,
      is_inclusive: false,
      is_default: false,
      metadata: {},
      created_at: daysAgo(110),
      updated_at: daysAgo(110),
    },
    {
      id: "txrate_ca_gst",
      tax_region_id: "txreg_ca",
      tax_category_id: "txcat_standard",
      name: "GST",
      rate_bps: 500,
      is_inclusive: false,
      is_default: true,
      metadata: {},
      created_at: daysAgo(90),
      updated_at: daysAgo(90),
    },
    {
      id: "txrate_de_vat",
      tax_region_id: "txreg_de",
      tax_category_id: "txcat_standard",
      name: "VAT",
      rate_bps: 1_900,
      is_inclusive: true,
      is_default: true,
      metadata: {},
      created_at: daysAgo(60),
      updated_at: daysAgo(60),
    },
  ];
}
