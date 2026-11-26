import type { PageResult, ReturnStatus, Timestamp } from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** A store-wide return row with the order context the table shows. */
export interface StoreReturnRow {
  id: string;
  order_id: string;
  order_display_number: number;
  customer_email: string;
  status: ReturnStatus;
  reason: string | null;
  item_count: number;
  created_at: Timestamp;
  updated_at: Timestamp;
}

/** One page of store-wide returns. */
export type StoreReturnPage = PageResult<StoreReturnRow>;

/** Query accepted by the store-wide return list. */
export interface StoreReturnQuery {
  limit?: number;
  offset?: number;
  status?: ReturnStatus;
}

async function returnsRequest<T>(
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
    throw new Error(apiErrorMessage(body, `Return request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists one page of returns across every order in the store, newest first. */
export function listStoreReturns(
  storeId: string,
  query: StoreReturnQuery = {},
  signal?: AbortSignal,
): Promise<StoreReturnPage> {
  const params = new URLSearchParams({
    limit: String(query.limit ?? 25),
    offset: String(query.offset ?? 0),
  });
  if (query.status) params.set("status", query.status);
  return returnsRequest<StoreReturnPage>(
    storeId,
    `/returns?${params.toString()}`,
    signal ? { signal } : {},
  );
}

export const RETURN_STATUSES: readonly ReturnStatus[] = [
  "draft",
  "requested",
  "partially_received",
  "received",
  "written_off",
  "canceled",
];

export const RETURN_STATUS_LABELS: Record<ReturnStatus, string> = {
  draft: "Draft",
  requested: "Requested",
  partially_received: "Partially received",
  received: "Received",
  written_off: "Written off",
  canceled: "Canceled",
};

export type ReturnStatusTone = "brand" | "warning" | "info" | "destructive" | "secondary";

export const RETURN_STATUS_TONES: Record<ReturnStatus, ReturnStatusTone> = {
  draft: "secondary",
  requested: "info",
  partially_received: "warning",
  received: "brand",
  written_off: "warning",
  canceled: "destructive",
};

/** Merchant-facing status label; unknown values fall back to the raw value. */
export function returnStatusLabel(status: string): string {
  return RETURN_STATUS_LABELS[status as ReturnStatus] ?? status;
}

/** Badge tone for a status, defaulting to secondary for unknown values. */
export function returnStatusTone(status: string): ReturnStatusTone {
  return RETURN_STATUS_TONES[status as ReturnStatus] ?? "secondary";
}

/** Status filter options for the table; `all` clears the filter. */
export function returnFilterOptions(): Array<{
  value: ReturnStatus | "all";
  label: string;
}> {
  return [
    { value: "all", label: "All" },
    ...RETURN_STATUSES.map((status) => ({
      value: status,
      label: RETURN_STATUS_LABELS[status],
    })),
  ];
}
