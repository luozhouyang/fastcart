import type { CurrencyCode, Metadata, PageResult, Timestamp } from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";
import { formatMinorAmount } from "@/lib/pricing";

export type CampaignBudgetType = "usage" | "spend";

/** One campaign as returned by `/api/v1/admin/campaigns`. */
export interface Campaign {
  id: string;
  name: string;
  description: string | null;
  budget_type: CampaignBudgetType;
  currency_code: CurrencyCode | null;
  budget_limit: number | null;
  budget_used: number;
  starts_at: Timestamp | null;
  ends_at: Timestamp | null;
  metadata: Metadata;
  created_at: Timestamp;
  updated_at: Timestamp;
}

/** Body accepted when a campaign is created. */
export interface CreateCampaignInput {
  name: string;
  description: string | null;
  budget_type: CampaignBudgetType;
  currency_code: string | null;
  budget_limit: number | null;
  starts_at: Timestamp | null;
  ends_at: Timestamp | null;
}

/**
 * Body accepted when a campaign is patched. Nullable fields are always sent so
 * the operator can clear a budget or a date window.
 */
export interface UpdateCampaignInput {
  name?: string;
  description?: string | null;
  budget_type?: CampaignBudgetType;
  currency_code?: string | null;
  budget_limit?: number | null;
  starts_at?: Timestamp | null;
  ends_at?: Timestamp | null;
}

export interface CampaignQuery {
  limit?: number;
  offset?: number;
}

function queryString(query: Record<string, string | number | undefined>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined) continue;
    params.set(key, String(value));
  }
  const encoded = params.toString();
  return encoded.length > 0 ? `?${encoded}` : "";
}

async function campaignRequest<T>(
  storeId: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("x-store-id", storeId);
  const csrf = readCookie("fc_csrf");
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/admin/campaigns${path}`, {
    ...init,
    credentials: "include",
    headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(apiErrorMessage(body, `Campaign request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function listCampaigns(storeId: string, query: CampaignQuery = {}, signal?: AbortSignal) {
  const path = queryString({ limit: query.limit, offset: query.offset });
  return campaignRequest<PageResult<Campaign>>(storeId, path, signal ? { signal } : {});
}

export function getCampaign(storeId: string, campaignId: string, signal?: AbortSignal) {
  return campaignRequest<Campaign>(storeId, `/${encodeURIComponent(campaignId)}`, {
    ...(signal ? { signal } : {}),
  });
}

export function createCampaign(storeId: string, input: CreateCampaignInput, signal?: AbortSignal) {
  return campaignRequest<Campaign>(storeId, "", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

export function updateCampaign(
  storeId: string,
  campaignId: string,
  input: UpdateCampaignInput,
  signal?: AbortSignal,
) {
  return campaignRequest<Campaign>(storeId, `/${encodeURIComponent(campaignId)}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

export function deleteCampaign(storeId: string, campaignId: string, signal?: AbortSignal) {
  return campaignRequest<void>(storeId, `/${encodeURIComponent(campaignId)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/** Lists the promotion ids linked to one campaign. */
export function listCampaignPromotions(storeId: string, campaignId: string, signal?: AbortSignal) {
  return campaignRequest<string[]>(storeId, `/${encodeURIComponent(campaignId)}/promotions`, {
    ...(signal ? { signal } : {}),
  });
}

export function linkCampaignPromotion(
  storeId: string,
  campaignId: string,
  promotionId: string,
  signal?: AbortSignal,
) {
  return campaignRequest<void>(storeId, `/${encodeURIComponent(campaignId)}/promotions`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ promotion_id: promotionId }),
    ...(signal ? { signal } : {}),
  });
}

export function unlinkCampaignPromotion(
  storeId: string,
  campaignId: string,
  promotionId: string,
  signal?: AbortSignal,
) {
  return campaignRequest<void>(
    storeId,
    `/${encodeURIComponent(campaignId)}/promotions/${encodeURIComponent(promotionId)}`,
    { method: "DELETE", ...(signal ? { signal } : {}) },
  );
}

export const CAMPAIGN_BUDGET_TYPES: readonly CampaignBudgetType[] = ["usage", "spend"];

export const CAMPAIGN_BUDGET_LABELS: Record<CampaignBudgetType, string> = {
  usage: "Usage",
  spend: "Spend",
};

/**
 * Spend campaigns need both a currency and a positive budget; usage campaigns
 * need a positive budget but no currency.
 */
export function campaignBudgetProblem(
  budgetType: CampaignBudgetType,
  currencyCode: string | null,
  budgetLimit: number | null,
): string | null {
  if (budgetLimit !== null && (!Number.isInteger(budgetLimit) || budgetLimit <= 0)) {
    return "Budget must be a positive whole number.";
  }
  if (budgetType === "spend") {
    if (currencyCode === null) return "A spend budget requires a currency.";
    if (budgetLimit === null) return "A spend budget requires a limit.";
  }
  return null;
}

/** Human-readable consumed budget, using money for spend campaigns. */
export function campaignBudgetLabel(campaign: Campaign): string {
  const { budget_type, budget_limit, budget_used, currency_code } = campaign;
  if (budget_type === "spend") {
    const currency = currency_code ?? "USD";
    if (budget_limit === null) return `${formatMinorAmount(budget_used, currency)} spent`;
    return `${formatMinorAmount(budget_used, currency)} of ${formatMinorAmount(
      budget_limit,
      currency,
    )}`;
  }
  if (budget_limit === null) return `${budget_used} uses`;
  return `${budget_used} of ${budget_limit}`;
}

/** Share of the campaign budget consumed so far, from 0 to 1. */
export function campaignBudgetFraction(
  budgetUsed: number,
  budgetLimit: number | null,
): number | null {
  if (budgetLimit === null || budgetLimit <= 0) return null;
  return Math.min(1, Math.max(0, budgetUsed / budgetLimit));
}

/** Promotions that may still be linked, in stable name order. */
export function linkablePromotions<T extends { id: string; title: string }>(
  promotions: readonly T[],
  linkedIds: readonly string[],
): T[] {
  const linked = new Set(linkedIds);
  return promotions
    .filter((promotion) => !linked.has(promotion.id))
    .sort(
      (left, right) => left.title.localeCompare(right.title) || left.id.localeCompare(right.id),
    );
}
