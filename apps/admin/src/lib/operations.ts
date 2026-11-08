import type { ShippingOption } from "@fastcart/shared-types";

import { formatMinorInput, parsePriceAmount } from "@/lib/pricing";

const COUNTRY_PATTERN = /^[A-Z]{2}$/;
const COUNTRY_SEPARATORS = /[\s,;]+/;
const PERCENT_PATTERN = /^\d+(?:\.\d{1,2})?$/;

/**
 * A shipping option as returned by the API.
 *
 * The backend exposes the fulfillment provider fields, but `shared-types` has
 * not caught up yet; keep the extra fields optional so both shapes typecheck.
 */
export type ShippingOptionRecord = ShippingOption & {
  provider_id?: string;
  provider_data?: unknown;
  fulfillment_set_id?: string | null;
};

/** Basis points in one percent. Tax rates are stored as `rate_bps`. */
export const BPS_PER_PERCENT = 100;

/**
 * Highest rate the admin accepts: 100%.
 *
 * The tax module stores rates in basis points where 10,000 is 100%. Its
 * `rate_bps` validation bound is wider (100,000), so the admin applies the
 * semantic 100% cap when converting operator-entered percentages.
 */
export const MAX_RATE_BPS = 10_000;

export interface CountryParseResult {
  /** Codes in their normalized, de-duplicated form, in input order. */
  codes: string[];
  /** Tokens that are not two-letter country codes. */
  invalid: string[];
  /** Codes that were already present or repeated in the input. */
  duplicates: string[];
}

/**
 * Normalizes a single ISO 3166-1 alpha-2 country code.
 *
 * The API validates codes against the full ISO list; the admin normalizes the
 * shape so obvious typos are caught before a request is sent.
 */
export function normalizeCountryCode(value: string): string | null {
  const candidate = value.trim().toUpperCase();
  return COUNTRY_PATTERN.test(candidate) ? candidate : null;
}

/** Adds comma/space separated country codes to an existing list. */
export function addCountryCodes(existing: readonly string[], input: string): CountryParseResult {
  const codes = [...existing];
  const invalid: string[] = [];
  const duplicates: string[] = [];

  for (const token of input.split(COUNTRY_SEPARATORS)) {
    if (token.trim().length === 0) continue;
    const code = normalizeCountryCode(token);
    if (code === null) {
      invalid.push(token.trim());
      continue;
    }
    if (codes.includes(code)) {
      duplicates.push(code);
      continue;
    }
    codes.push(code);
  }

  return { codes, invalid, duplicates };
}

/** Parses a standalone country list, for example `us, ca`. */
export function parseCountryCodes(input: string): CountryParseResult {
  return addCountryCodes([], input);
}

/** Joins country codes for a compact table cell. */
export function summarizeCountryCodes(codes: readonly string[], max = 4): string {
  if (codes.length === 0) return "—";
  if (codes.length <= max) return codes.join(", ");
  return `${codes.slice(0, max).join(", ")} +${codes.length - max}`;
}

/**
 * Parses a comma/space separated payment provider list.
 *
 * Provider identifiers are free-form; values are trimmed, de-duplicated and
 * otherwise kept as typed so custom provider ids keep their casing.
 */
export function parsePaymentProviders(input: string): string[] {
  const providers: string[] = [];
  for (const token of input.split(COUNTRY_SEPARATORS)) {
    const value = token.trim();
    if (value.length === 0 || providers.includes(value)) continue;
    providers.push(value);
  }
  return providers;
}

/** Normalizes an optional province/state code; empty input clears it. */
export function normalizeProvince(value: string): string | null {
  const trimmed = value.trim().toUpperCase();
  return trimmed.length === 0 ? null : trimmed;
}

/** Converts a stored basis-point rate to a percentage. */
export function bpsToPercent(bps: number): number {
  return bps / BPS_PER_PERCENT;
}

/** Converts a percentage to integer basis points. */
export function percentToBps(percent: number): number {
  return Math.round(percent * BPS_PER_PERCENT);
}

export type PercentBpsResult = { ok: true; bps: number } | { ok: false; message: string };

/**
 * Parses a tax percentage typed by an operator (for example `8.25`) into the
 * integer basis points the API stores. Up to two decimal places are accepted.
 */
export function parsePercentToBps(input: string): PercentBpsResult {
  const value = input.trim().replace(/%$/, "").trim();
  if (value.length === 0) {
    return { ok: false, message: "Enter a tax rate percentage." };
  }
  if (!PERCENT_PATTERN.test(value)) {
    return {
      ok: false,
      message: "Enter a percentage with up to two decimals, for example 8.25.",
    };
  }

  const percent = Number(value);
  if (!Number.isFinite(percent)) {
    return { ok: false, message: "Enter a valid percentage." };
  }

  const bps = percentToBps(percent);
  if (!Number.isSafeInteger(bps) || bps > MAX_RATE_BPS) {
    return { ok: false, message: "Tax rates cannot exceed 100%." };
  }

  return { ok: true, bps };
}

/** Formats a stored basis-point rate as a compact percentage string. */
export function formatRatePercent(bps: number): string {
  const percent = bpsToPercent(bps);
  if (Number.isInteger(percent)) return `${percent.toFixed(0)}%`;
  return `${percent.toFixed(2).replace(/0+$/, "").replace(/\.$/, "")}%`;
}

/** Prefills a tax percentage input from a stored rate. */
export function formatRateInput(bps: number): string {
  return formatRatePercent(bps).replace(/%$/, "");
}

export type OptionalAmountResult =
  { ok: true; minor: number | null } | { ok: false; message: string };

/**
 * Parses an optional major-unit amount (shipping subtotal bounds) into minor
 * units. An empty value means "no bound" and becomes `null`.
 */
export function parseOptionalMinorAmount(input: string): OptionalAmountResult {
  if (input.trim().length === 0) return { ok: true, minor: null };
  const parsed = parsePriceAmount(input);
  if (!parsed.ok) return { ok: false, message: parsed.message };
  return { ok: true, minor: parsed.minor };
}

/** Prefills an optional amount input from minor units. */
export function formatOptionalMinorAmount(minor: number | null): string {
  return minor === null ? "" : formatMinorInput(minor);
}

/**
 * Masks a publishable key prefix for list views.
 *
 * The API never returns the full secret after creation, so the admin appends a
 * visual mask to the stored prefix and only reveals the token once.
 */
export function maskPublishableKey(prefix: string): string {
  const trimmed = prefix.trim();
  return trimmed.length === 0 ? "••••" : `${trimmed}••••`;
}
