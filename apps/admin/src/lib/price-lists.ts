import type {
  CreatePriceListRequest,
  PageResult,
  Price,
  PriceList,
  PriceListStatus,
  PriceListType,
  Timestamp,
  UpdatePriceListRequest,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

export type { PriceListStatus, PriceListType };

/** A price record scoped to one price list. */
export type PriceListPrice = Price;

/** Price list fields used when a list is edited. */
export interface PriceListInput {
  title: string;
  description: string | null;
  price_type: PriceListType;
  status: PriceListStatus;
  ends_at: Timestamp | null;
}

/** Query accepted by the price list collection. */
export interface PriceListQuery {
  limit?: number;
  offset?: number;
}

/** Query accepted by the price collection for one list. */
export interface PriceQuery {
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

async function priceListRequest<T>(
  storeId: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("x-store-id", storeId);
  const csrf = readCookie("fc_csrf");
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/admin${path}`, {
    ...init,
    credentials: "include",
    headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(apiErrorMessage(body, `Price list request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function listPriceLists(storeId: string, query: PriceListQuery = {}, signal?: AbortSignal) {
  const path = `/price-lists${queryString({ limit: query.limit, offset: query.offset })}`;
  return priceListRequest<PageResult<PriceList>>(storeId, path, signal ? { signal } : {});
}

export function createPriceList(storeId: string, input: PriceListInput, signal?: AbortSignal) {
  const body: CreatePriceListRequest = {
    title: input.title,
    description: input.description,
    price_type: input.price_type,
    status: input.status,
    ends_at: input.ends_at,
  };
  return priceListRequest<PriceList>(storeId, "/price-lists", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    ...(signal ? { signal } : {}),
  });
}

export function updatePriceList(
  storeId: string,
  priceListId: string,
  input: Partial<PriceListInput>,
  signal?: AbortSignal,
) {
  const body: UpdatePriceListRequest = {};
  if (input.title !== undefined) body.title = input.title;
  if (input.description !== undefined) body.description = input.description;
  if (input.price_type !== undefined) body.price_type = input.price_type;
  if (input.status !== undefined) body.status = input.status;
  if (input.ends_at !== undefined) body.ends_at = input.ends_at;
  return priceListRequest<PriceList>(storeId, `/price-lists/${encodeURIComponent(priceListId)}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    ...(signal ? { signal } : {}),
  });
}

export function deletePriceList(storeId: string, priceListId: string, signal?: AbortSignal) {
  return priceListRequest<void>(storeId, `/price-lists/${encodeURIComponent(priceListId)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/** Lists the prices attached to one price list. */
export function listPriceListPrices(
  storeId: string,
  priceListId: string,
  query: PriceQuery = {},
  signal?: AbortSignal,
) {
  const path = `/prices${queryString({
    price_list_id: priceListId,
    limit: query.limit,
    offset: query.offset,
  })}`;
  return priceListRequest<PageResult<PriceListPrice>>(storeId, path, signal ? { signal } : {});
}

export const PRICE_LIST_TYPES: readonly PriceListType[] = ["sale", "override"];
export const PRICE_LIST_STATUSES: readonly PriceListStatus[] = ["draft", "active"];

export const PRICE_LIST_TYPE_LABELS: Record<PriceListType, string> = {
  sale: "Sale",
  override: "Override",
};

export const PRICE_LIST_STATUS_LABELS: Record<PriceListStatus, string> = {
  draft: "Draft",
  active: "Active",
};

/** A sale list discounts a base price; an override list replaces it outright. */
export function priceListTypeHint(priceType: PriceListType): string {
  return priceType === "sale"
    ? "Prices are discounts compared against the base price."
    : "Prices replace the base price unconditionally.";
}

/** Filters an unsorted price page down to the rows of one list. */
export function pricesForList(
  prices: readonly PriceListPrice[],
  priceListId: string,
): PriceListPrice[] {
  return prices.filter((price) => price.price_list_id === priceListId);
}

/** True when the list has ended at `now` (Unix seconds). */
export function priceListEnded(endsAt: Timestamp | null, now: number): boolean {
  return endsAt !== null && now >= endsAt;
}
