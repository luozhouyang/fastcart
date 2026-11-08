import { describe, expect, it } from "vitest";

import {
  couponCodeError,
  formatPercentageBasisPoints,
  formatPromotionValue,
  normalizeCouponCode,
  parsePromotionValue,
  promotionChannelLabel,
  promotionRuleSummary,
  promotionStatusLabel,
  promotionTypeLabel,
  promotionUsageLabel,
  promotionValueToInput,
} from "@/lib/promotions";

describe("promotion value display", () => {
  it("renders percentage basis points as percentages", () => {
    expect(formatPercentageBasisPoints(1500)).toBe("15%");
    expect(formatPercentageBasisPoints(1250)).toBe("12.5%");
    expect(formatPercentageBasisPoints(1234)).toBe("12.34%");
    expect(formatPercentageBasisPoints(10000)).toBe("100%");
  });

  it("renders fixed amounts from minor units", () => {
    expect(formatPromotionValue("fixed_amount", 1250)).toBe("$12.50");
    expect(formatPromotionValue("fixed_amount", 1250, "EUR")).toBe("€12.50");
    expect(formatPromotionValue("fixed_amount", 2000)).toBe("$20.00");
  });

  it("renders free shipping without an amount", () => {
    expect(formatPromotionValue("free_shipping", 0)).toBe("Free shipping");
  });
});

describe("promotion value inputs", () => {
  it("converts stored values back into dialog inputs", () => {
    expect(promotionValueToInput("percentage", 1250)).toBe("12.5");
    expect(promotionValueToInput("percentage", 1500)).toBe("15");
    expect(promotionValueToInput("percentage", 0)).toBe("0");
    expect(promotionValueToInput("fixed_amount", 1250)).toBe("12.50");
    expect(promotionValueToInput("free_shipping", 0)).toBe("0");
  });

  it("parses percentages into basis points", () => {
    expect(parsePromotionValue("percentage", "12.5")).toEqual({ ok: true, value: 1250 });
    expect(parsePromotionValue("percentage", "15")).toEqual({ ok: true, value: 1500 });
    expect(parsePromotionValue("percentage", "100")).toEqual({ ok: true, value: 10000 });
    expect(parsePromotionValue("percentage", "0")).toEqual({ ok: true, value: 0 });
  });

  it("rejects empty, malformed and out-of-range percentages", () => {
    expect(parsePromotionValue("percentage", "")).toMatchObject({ ok: false });
    expect(parsePromotionValue("percentage", "abc")).toMatchObject({ ok: false });
    expect(parsePromotionValue("percentage", "-1")).toMatchObject({ ok: false });
    expect(parsePromotionValue("percentage", "100.01")).toMatchObject({ ok: false });
  });

  it("parses fixed amounts into minor units", () => {
    expect(parsePromotionValue("fixed_amount", "12.50")).toEqual({ ok: true, value: 1250 });
    expect(parsePromotionValue("fixed_amount", "12")).toEqual({ ok: true, value: 1200 });
    expect(parsePromotionValue("fixed_amount", "0")).toEqual({ ok: true, value: 0 });
  });

  it("rejects negative and malformed fixed amounts", () => {
    expect(parsePromotionValue("fixed_amount", "-1")).toMatchObject({ ok: false });
    expect(parsePromotionValue("fixed_amount", "")).toMatchObject({ ok: false });
    expect(parsePromotionValue("fixed_amount", "12,50")).toMatchObject({ ok: false });
  });

  it("always stores free shipping as zero", () => {
    expect(parsePromotionValue("free_shipping", "anything")).toEqual({ ok: true, value: 0 });
  });
});

describe("promotion labels", () => {
  it("labels every backend status", () => {
    expect(promotionStatusLabel("draft")).toBe("Draft");
    expect(promotionStatusLabel("active")).toBe("Active");
    expect(promotionStatusLabel("expired")).toBe("Expired");
    expect(promotionStatusLabel("disabled")).toBe("Disabled");
  });

  it("labels every backend type", () => {
    expect(promotionTypeLabel("percentage")).toBe("Percentage");
    expect(promotionTypeLabel("fixed_amount")).toBe("Fixed amount");
    expect(promotionTypeLabel("free_shipping")).toBe("Free shipping");
  });

  it("describes automatic and coupon-based promotions", () => {
    expect(promotionChannelLabel({ is_automatic: true, code: null })).toBe("Automatic");
    expect(promotionChannelLabel({ is_automatic: false, code: "SAVE10" })).toBe("Code SAVE10");
    expect(promotionChannelLabel({ is_automatic: false, code: null })).toBe("Coupon");
  });

  it("formats usage against an optional limit", () => {
    expect(promotionUsageLabel(0, null)).toBe("0 uses");
    expect(promotionUsageLabel(1, null)).toBe("1 use");
    expect(promotionUsageLabel(42, 500)).toBe("42 / 500");
  });

  it("summarises rules for the detail drawer", () => {
    expect(
      promotionRuleSummary({
        attribute: "minimum_order_subtotal",
        operator: "gte",
        value: "7500",
      }),
    ).toBe("Minimum order subtotal at least 7500");
  });
});

describe("coupon codes", () => {
  it("normalizes codes the way the backend does", () => {
    expect(normalizeCouponCode(" save10 ")).toBe("SAVE10");
    expect(normalizeCouponCode("Welcome-15")).toBe("WELCOME-15");
  });

  it("accepts empty and well-formed codes", () => {
    expect(couponCodeError("")).toBeNull();
    expect(couponCodeError("SAVE-10_2")).toBeNull();
  });

  it("rejects invalid characters and overlong codes", () => {
    expect(couponCodeError("SAVE 10")).toMatch(/letters/);
    expect(couponCodeError("SAVE.10")).toMatch(/letters/);
    expect(couponCodeError("A".repeat(65))).toMatch(/64/);
  });
});
