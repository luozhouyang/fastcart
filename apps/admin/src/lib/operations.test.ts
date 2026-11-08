import { describe, expect, it } from "vitest";

import {
  addCountryCodes,
  bpsToPercent,
  formatOptionalMinorAmount,
  formatRateInput,
  formatRatePercent,
  maskPublishableKey,
  normalizeCountryCode,
  normalizeProvince,
  parseCountryCodes,
  parseOptionalMinorAmount,
  parsePaymentProviders,
  parsePercentToBps,
  percentToBps,
  summarizeCountryCodes,
} from "@/lib/operations";

describe("country normalization", () => {
  it("uppercases and trims two-letter codes", () => {
    expect(normalizeCountryCode(" us ")).toBe("US");
    expect(normalizeCountryCode("ca")).toBe("CA");
  });

  it("rejects malformed codes", () => {
    expect(normalizeCountryCode("USA")).toBeNull();
    expect(normalizeCountryCode("U1")).toBeNull();
    expect(normalizeCountryCode("")).toBeNull();
  });

  it("parses comma and space separated lists", () => {
    expect(parseCountryCodes("us, ca  gb;de")).toEqual({
      codes: ["US", "CA", "GB", "DE"],
      invalid: [],
      duplicates: [],
    });
  });

  it("reports invalid tokens and duplicates", () => {
    expect(parseCountryCodes("US, usa, US, 12")).toEqual({
      codes: ["US"],
      invalid: ["usa", "12"],
      duplicates: ["US"],
    });
  });

  it("adds to an existing list without mutating it", () => {
    const existing = ["US"];
    const result = addCountryCodes(existing, "CA us");
    expect(result.codes).toEqual(["US", "CA"]);
    expect(result.duplicates).toEqual(["US"]);
    expect(existing).toEqual(["US"]);
  });

  it("summarizes long lists", () => {
    expect(summarizeCountryCodes([])).toBe("—");
    expect(summarizeCountryCodes(["US", "CA"])).toBe("US, CA");
    expect(summarizeCountryCodes(["US", "CA", "GB", "DE", "FR"])).toBe("US, CA, GB, DE +1");
  });
});

describe("payment providers", () => {
  it("splits, trims and de-duplicates provider identifiers", () => {
    expect(parsePaymentProviders("manual, stripe,manual")).toEqual(["manual", "stripe"]);
    expect(parsePaymentProviders("  ")).toEqual([]);
    expect(parsePaymentProviders("custom-pay")).toEqual(["custom-pay"]);
  });
});

describe("province normalization", () => {
  it("uppercases and clears empty values", () => {
    expect(normalizeProvince(" ca ")).toBe("CA");
    expect(normalizeProvince("")).toBeNull();
    expect(normalizeProvince("   ")).toBeNull();
  });
});

describe("tax rate conversion", () => {
  it("converts between basis points and percentages", () => {
    expect(bpsToPercent(825)).toBe(8.25);
    expect(bpsToPercent(0)).toBe(0);
    expect(percentToBps(8.25)).toBe(825);
    expect(percentToBps(7.1)).toBe(710);
    expect(percentToBps(0)).toBe(0);
  });

  it("parses operator-entered percentages", () => {
    expect(parsePercentToBps("8.25")).toEqual({ ok: true, bps: 825 });
    expect(parsePercentToBps(" 7 ")).toEqual({ ok: true, bps: 700 });
    expect(parsePercentToBps("0")).toEqual({ ok: true, bps: 0 });
    expect(parsePercentToBps("100")).toEqual({ ok: true, bps: 10_000 });
    expect(parsePercentToBps("8.25%")).toEqual({ ok: true, bps: 825 });
  });

  it("rejects missing, malformed and out-of-range percentages", () => {
    expect(parsePercentToBps("")).toMatchObject({ ok: false });
    expect(parsePercentToBps("8.255")).toMatchObject({ ok: false });
    expect(parsePercentToBps("abc")).toMatchObject({ ok: false });
    expect(parsePercentToBps("-1")).toMatchObject({ ok: false });
    expect(parsePercentToBps("100.01")).toMatchObject({ ok: false });
  });

  it("formats rates compactly and prefills inputs", () => {
    expect(formatRatePercent(800)).toBe("8%");
    expect(formatRatePercent(825)).toBe("8.25%");
    expect(formatRatePercent(8_250)).toBe("82.5%");
    expect(formatRatePercent(10_000)).toBe("100%");
    expect(formatRateInput(725)).toBe("7.25");
    expect(formatRateInput(2_000)).toBe("20");
  });
});

describe("optional minor amounts", () => {
  it("treats empty input as no bound", () => {
    expect(parseOptionalMinorAmount("")).toEqual({ ok: true, minor: null });
    expect(parseOptionalMinorAmount("   ")).toEqual({ ok: true, minor: null });
  });

  it("converts major units to minor units", () => {
    expect(parseOptionalMinorAmount("12.5")).toEqual({ ok: true, minor: 1_250 });
    expect(parseOptionalMinorAmount("0")).toEqual({ ok: true, minor: 0 });
  });

  it("rejects invalid amounts", () => {
    expect(parseOptionalMinorAmount("12,50")).toMatchObject({ ok: false });
    expect(parseOptionalMinorAmount("-1")).toMatchObject({ ok: false });
  });

  it("prefills inputs from minor units", () => {
    expect(formatOptionalMinorAmount(1_250)).toBe("12.50");
    expect(formatOptionalMinorAmount(null)).toBe("");
  });
});

describe("publishable key masking", () => {
  it("appends a mask to the stored prefix", () => {
    expect(maskPublishableKey("pk_abc12345")).toBe("pk_abc12345••••");
    expect(maskPublishableKey("  ")).toBe("••••");
  });
});
