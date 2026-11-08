import type {
  Promotion,
  PromotionRule,
  PromotionRuleAttribute,
  PromotionRuleOperator,
  PromotionStatus,
  PromotionType,
} from "@fastcart/shared-types";

import { money } from "@/lib/format";

export const PROMOTION_STATUSES = [
  "active",
  "draft",
  "expired",
  "disabled",
] as const satisfies readonly PromotionStatus[];

export const PROMOTION_TYPES = [
  "percentage",
  "fixed_amount",
  "free_shipping",
] as const satisfies readonly PromotionType[];

export const PROMOTION_STATUS_LABELS: Record<PromotionStatus, string> = {
  draft: "Draft",
  active: "Active",
  expired: "Expired",
  disabled: "Disabled",
};

export const PROMOTION_TYPE_LABELS: Record<PromotionType, string> = {
  percentage: "Percentage",
  fixed_amount: "Fixed amount",
  free_shipping: "Free shipping",
};

/** Badge tones shared by the table and the detail drawer. */
export type PromotionStatusTone = "brand" | "secondary" | "warning" | "destructive";

export const PROMOTION_STATUS_TONES: Record<PromotionStatus, PromotionStatusTone> = {
  active: "brand",
  draft: "secondary",
  expired: "warning",
  disabled: "destructive",
};

export function promotionStatusLabel(status: PromotionStatus): string {
  return PROMOTION_STATUS_LABELS[status];
}

export function promotionTypeLabel(promotionType: PromotionType): string {
  return PROMOTION_TYPE_LABELS[promotionType];
}

const percentFormatter = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 2,
});

const decimalPattern = /^-?(?:\d+(?:\.\d*)?|\.\d+)$/;

/** Formats the API's percentage basis points as a human percentage. */
export function formatPercentageBasisPoints(basisPoints: number): string {
  return `${percentFormatter.format(basisPoints / 100)}%`;
}

/**
 * Formats a promotion value for display.
 *
 * The backend stores percentages in basis points and fixed amounts in minor
 * currency units, while `money` accepts major units.
 */
export function formatPromotionValue(
  promotionType: PromotionType,
  value: number,
  currency = "USD",
): string {
  if (promotionType === "percentage") return formatPercentageBasisPoints(value);
  if (promotionType === "fixed_amount") return money(value / 100, currency);
  return "Free shipping";
}

/** Formats a stored value for the edit dialog's text input. */
export function promotionValueToInput(promotionType: PromotionType, value: number): string {
  if (promotionType === "percentage") {
    const percent = value / 100;
    return percent.toFixed(2).replace(/0+$/, "").replace(/\.$/, "");
  }
  if (promotionType === "fixed_amount") return (value / 100).toFixed(2);
  return "0";
}

export type PromotionValueParseResult = { ok: true; value: number } | { ok: false; error: string };

/**
 * Converts a dialog input into the backend's stored representation.
 *
 * Percentages are entered as percent (for example `12.5`) and stored as basis
 * points (`1250`). Fixed amounts are entered as major units and stored as
 * minor units.
 */
export function parsePromotionValue(
  promotionType: PromotionType,
  raw: string,
): PromotionValueParseResult {
  if (promotionType === "free_shipping") return { ok: true, value: 0 };

  const trimmed = raw.trim();
  const parsed = decimalPattern.test(trimmed) ? Number(trimmed) : null;
  if (parsed === null || !Number.isFinite(parsed)) {
    return {
      ok: false,
      error:
        promotionType === "percentage"
          ? "Enter a percentage, for example 15."
          : "Enter an amount, for example 12.50.",
    };
  }

  if (parsed < 0) {
    return {
      ok: false,
      error:
        promotionType === "percentage"
          ? "Enter a percentage between 0 and 100."
          : "Enter an amount of zero or more.",
    };
  }

  if (promotionType === "percentage" && parsed > 100) {
    return { ok: false, error: "Enter a percentage between 0 and 100." };
  }

  const value = Math.round(parsed * 100);
  if (!Number.isSafeInteger(value)) {
    return {
      ok: false,
      error:
        promotionType === "percentage"
          ? "Enter a percentage between 0 and 100."
          : "Enter a smaller amount.",
    };
  }

  return { ok: true, value };
}

/** Trims and uppercases a coupon code the way the backend does. */
export function normalizeCouponCode(code: string): string {
  return code.trim().toUpperCase();
}

/** Returns a validation message, or null when the code is empty/valid. */
export function couponCodeError(raw: string): string | null {
  const code = normalizeCouponCode(raw);
  if (code.length === 0) return null;
  if (code.length > 64) return "Coupon codes cannot be longer than 64 characters.";
  if (!/^[A-Z0-9_-]+$/.test(code)) {
    return "Use only letters, digits, hyphens and underscores.";
  }
  return null;
}

/** A compact label for the automatic/coupon channel column. */
export function promotionChannelLabel(promotion: Pick<Promotion, "is_automatic" | "code">): string {
  if (promotion.is_automatic) return "Automatic";
  return promotion.code ? `Code ${promotion.code}` : "Coupon";
}

/** Formats usage against an optional limit. */
export function promotionUsageLabel(usageCount: number, usageLimit: number | null): string {
  if (usageLimit === null) {
    return `${usageCount.toLocaleString("en-US")} ${usageCount === 1 ? "use" : "uses"}`;
  }
  return `${usageCount.toLocaleString("en-US")} / ${usageLimit.toLocaleString("en-US")}`;
}

const RULE_ATTRIBUTE_LABELS: Record<PromotionRuleAttribute, string> = {
  minimum_order_subtotal: "Minimum order subtotal",
  product_id: "Product",
  variant_id: "Variant",
  category_id: "Category",
  customer_group_id: "Customer group",
  currency: "Currency",
  first_order: "First order",
};

const RULE_OPERATOR_LABELS: Record<PromotionRuleOperator, string> = {
  eq: "is",
  in: "is one of",
  gte: "at least",
  lte: "at most",
};

/** Human-readable one-line summary of a promotion rule. */
export function promotionRuleSummary(
  rule: Pick<PromotionRule, "attribute" | "operator" | "value">,
): string {
  return `${RULE_ATTRIBUTE_LABELS[rule.attribute]} ${RULE_OPERATOR_LABELS[rule.operator]} ${rule.value}`;
}
