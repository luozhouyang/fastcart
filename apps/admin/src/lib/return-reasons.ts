import type {
  CreateReturnReasonRequest,
  ReturnReason,
  UpdateReturnReasonRequest,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/**
 * Configured after-sales reasons.
 *
 * The API requires `orders.read`/`orders.write`, so the reason editor lives
 * beside the after-sales surfaces rather than in the general settings.
 */
async function reasonsRequest<T>(
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
    throw new Error(apiErrorMessage(body, `Return reason request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists every configured reason, oldest first. */
export function listReturnReasons(storeId: string, signal?: AbortSignal): Promise<ReturnReason[]> {
  return reasonsRequest<ReturnReason[]>(storeId, "/return-reasons", signal ? { signal } : {});
}

/** Creates a reason. `value` is unique per store; `label` is snapshotted. */
export function createReturnReason(
  storeId: string,
  input: CreateReturnReasonRequest,
  signal?: AbortSignal,
): Promise<ReturnReason> {
  return reasonsRequest<ReturnReason>(storeId, "/return-reasons", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Renames or re-describes a reason. `description: null` clears it. */
export function updateReturnReason(
  storeId: string,
  reasonId: string,
  input: UpdateReturnReasonRequest,
  signal?: AbortSignal,
): Promise<ReturnReason> {
  return reasonsRequest<ReturnReason>(storeId, `/return-reasons/${encodeURIComponent(reasonId)}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Deletes a reason. Existing after-sales records keep their snapshotted label. */
export function deleteReturnReason(
  storeId: string,
  reasonId: string,
  signal?: AbortSignal,
): Promise<void> {
  return reasonsRequest<void>(storeId, `/return-reasons/${encodeURIComponent(reasonId)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/** Validates the reason form before a request is sent. */
export function returnReasonInputError(input: { value: string; label: string }): string | null {
  const value = input.value.trim();
  const label = input.label.trim();
  if (value.length === 0) return "Enter a value such as damaged.";
  if (value.length > 120) return "Values cannot be longer than 120 characters.";
  if (label.length === 0) return "Enter a label shown to operators.";
  if (label.length > 200) return "Labels cannot be longer than 200 characters.";
  return null;
}
