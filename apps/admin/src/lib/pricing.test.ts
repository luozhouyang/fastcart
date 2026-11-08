import { describe, expect, it } from "vitest";

import {
  formatMinorAmount,
  formatMinorInput,
  isValidCurrency,
  majorToMinor,
  minorToMajor,
  normalizeCurrency,
  parsePriceAmount,
} from "@/lib/pricing";

describe("price amount conversion", () => {
  it("rounds major units to integer minor units", () => {
    expect(majorToMinor(19.99)).toBe(1999);
    expect(majorToMinor(19)).toBe(1900);
    expect(majorToMinor(0.1)).toBe(10);
    expect(majorToMinor(19.999)).toBe(2000);
    expect(minorToMajor(1999)).toBe(19.99);
  });

  it("parses amounts typed in major units", () => {
    expect(parsePriceAmount("19.99")).toEqual({ ok: true, minor: 1999 });
    expect(parsePriceAmount(" 19 ")).toEqual({ ok: true, minor: 1900 });
    expect(parsePriceAmount("0")).toEqual({ ok: true, minor: 0 });
  });

  it("rejects missing, invalid and negative amounts", () => {
    expect(parsePriceAmount("")).toMatchObject({ ok: false });
    expect(parsePriceAmount("   ")).toMatchObject({ ok: false });
    expect(parsePriceAmount("abc")).toMatchObject({ ok: false });
    expect(parsePriceAmount("19,99")).toMatchObject({ ok: false });
    expect(parsePriceAmount("1e3")).toMatchObject({ ok: false });
    expect(parsePriceAmount("-1")).toMatchObject({ ok: false });
  });

  it("prefills edit inputs with two decimals", () => {
    expect(formatMinorInput(1999)).toBe("19.99");
    expect(formatMinorInput(1900)).toBe("19.00");
  });
});

describe("price formatting", () => {
  it("formats minor units as money", () => {
    expect(formatMinorAmount(1999, "USD")).toBe("$19.99");
    expect(formatMinorAmount(1999, "EUR")).toBe("€19.99");
  });

  it("falls back for unsupported currency codes", () => {
    expect(formatMinorAmount(1999, "not-a-code")).toBe("$19.99");
  });
});

describe("currency normalization", () => {
  it("uppercases valid three-letter codes", () => {
    expect(normalizeCurrency(" eur ")).toBe("EUR");
    expect(normalizeCurrency("usd")).toBe("USD");
  });

  it("falls back for invalid codes", () => {
    expect(normalizeCurrency("", "USD")).toBe("USD");
    expect(normalizeCurrency("US", "USD")).toBe("USD");
    expect(isValidCurrency("GBP")).toBe(true);
    expect(isValidCurrency("GB")).toBe(false);
  });
});
