import type { Store } from "@fastcart/shared-types";

/** The store shown in review mode. It mirrors the design dataset. */
export const demoStore: Store = {
  id: "demo-store",
  name: "Halcyon Supply",
  slug: "halcyon-supply",
  status: "active",
  primary_domain: "halcyonsupply.com",
  locale: "en-US",
  currency_code: "USD",
  timezone: "America/New_York",
  created_at: Math.floor(new Date("2025-01-12T10:00:00Z").getTime() / 1000),
  updated_at: Math.floor(Date.now() / 1000),
};

export const DEMO_STORE_DOMAIN = demoStore.primary_domain ?? "halcyonsupply.com";
