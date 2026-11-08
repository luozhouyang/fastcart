import { money } from "@/lib/format";

const CURRENCY_PATTERN = /^[A-Z]{3}$/;
const AMOUNT_PATTERN = /^\d+(?:\.\d+)?$/;

export type PriceAmountResult = { ok: true; minor: number } | { ok: false; message: string };

/** Converts an amount in major units (e.g. 19.99) to integer minor units. */
export function majorToMinor(major: number): number {
  return Math.round(major * 100);
}

/** Converts integer minor units back to major units for display or editing. */
export function minorToMajor(minor: number): number {
  return minor / 100;
}

/**
 * Parses a price typed in major units. The API stores prices as integer minor
 * units, so every amount is rounded the same way before it is sent.
 */
export function parsePriceAmount(input: string): PriceAmountResult {
  const value = input.trim();
  if (value.length === 0) {
    return { ok: false, message: "Enter an amount." };
  }
  if (!AMOUNT_PATTERN.test(value)) {
    return { ok: false, message: "Enter a valid amount, for example 19.99." };
  }

  const major = Number(value);
  if (!Number.isFinite(major)) {
    return { ok: false, message: "Enter a valid amount." };
  }
  if (major < 0) {
    return { ok: false, message: "Amount cannot be negative." };
  }

  const minor = majorToMinor(major);
  if (!Number.isSafeInteger(minor)) {
    return { ok: false, message: "Amount is too large." };
  }

  return { ok: true, minor };
}

/** Normalizes an ISO currency code, falling back when the value is unusable. */
export function normalizeCurrency(value: string | null | undefined, fallback = "USD"): string {
  const candidate = (value ?? "").trim().toUpperCase();
  return CURRENCY_PATTERN.test(candidate) ? candidate : fallback;
}

export function isValidCurrency(value: string): boolean {
  return CURRENCY_PATTERN.test(value.trim().toUpperCase());
}

/** Formats integer minor units using the store currency. */
export function formatMinorAmount(minor: number, currency: string): string {
  const code = normalizeCurrency(currency);
  try {
    return money(minorToMajor(minor), code);
  } catch {
    return money(minorToMajor(minor), "USD");
  }
}

/** Value used to prefill an amount input from an API price. */
export function formatMinorInput(minor: number): string {
  return minorToMajor(minor).toFixed(2);
}
