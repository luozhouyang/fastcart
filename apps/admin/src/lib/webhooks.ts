import type {
  CreatedWebhookEndpoint,
  Metadata,
  RotatedWebhookSecret,
  WebhookDelivery,
  WebhookDeliveryStatus,
  WebhookEndpoint,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

export type { WebhookDelivery, WebhookDeliveryStatus, WebhookEndpoint };

/** Body accepted when an endpoint is registered. */
export interface CreateWebhookInput {
  url: string;
  events: string[];
  secret?: string | null;
  metadata?: Metadata | null;
}

/** Body accepted when an endpoint is patched. */
export interface UpdateWebhookInput {
  url?: string;
  events?: string[];
  is_active?: boolean;
}

export interface WebhookDeliveryQuery {
  limit?: number;
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

async function webhookRequest<T>(
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
    throw new Error(apiErrorMessage(body, `Webhook request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function listWebhooks(storeId: string, signal?: AbortSignal) {
  return webhookRequest<WebhookEndpoint[]>(storeId, "/webhooks", signal ? { signal } : {});
}

export function createWebhook(storeId: string, input: CreateWebhookInput, signal?: AbortSignal) {
  return webhookRequest<CreatedWebhookEndpoint>(storeId, "/webhooks", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

export function updateWebhook(
  storeId: string,
  endpointId: string,
  input: UpdateWebhookInput,
  signal?: AbortSignal,
) {
  return webhookRequest<WebhookEndpoint>(storeId, `/webhooks/${encodeURIComponent(endpointId)}`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

export function deleteWebhook(storeId: string, endpointId: string, signal?: AbortSignal) {
  return webhookRequest<void>(storeId, `/webhooks/${encodeURIComponent(endpointId)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/** Rotates the signing secret; the new secret is returned exactly once. */
export function rotateWebhookSecret(storeId: string, endpointId: string, signal?: AbortSignal) {
  return webhookRequest<RotatedWebhookSecret>(
    storeId,
    `/webhooks/${encodeURIComponent(endpointId)}/rotate-secret`,
    { method: "POST", ...(signal ? { signal } : {}) },
  );
}

export function listWebhookDeliveries(
  storeId: string,
  endpointId: string,
  query: WebhookDeliveryQuery = {},
  signal?: AbortSignal,
) {
  const path = `/webhooks/${encodeURIComponent(endpointId)}/deliveries${queryString({
    limit: query.limit,
  })}`;
  return webhookRequest<WebhookDelivery[]>(storeId, path, signal ? { signal } : {});
}

/** Requeues a delivery and returns its refreshed row. */
export function replayWebhookDelivery(storeId: string, deliveryId: string, signal?: AbortSignal) {
  return webhookRequest<WebhookDelivery>(
    storeId,
    `/webhook-deliveries/${encodeURIComponent(deliveryId)}/replay`,
    { method: "POST", ...(signal ? { signal } : {}) },
  );
}

export const WEBHOOK_DELIVERY_STATUSES: readonly WebhookDeliveryStatus[] = [
  "pending",
  "delivering",
  "delivered",
  "failed",
];

export const WEBHOOK_DELIVERY_LABELS: Record<WebhookDeliveryStatus, string> = {
  pending: "Pending",
  delivering: "Delivering",
  delivered: "Delivered",
  failed: "Failed",
};

export type WebhookDeliveryTone = "secondary" | "info" | "brand" | "destructive";

export const WEBHOOK_DELIVERY_TONES: Record<WebhookDeliveryStatus, WebhookDeliveryTone> = {
  pending: "secondary",
  delivering: "info",
  delivered: "brand",
  failed: "destructive",
};

/** A delivery may be replayed unless a worker is currently sending it. */
export function canReplayDelivery(status: WebhookDeliveryStatus): boolean {
  return status !== "delivering";
}

/** Normalizes the comma/space separated event list typed into the dialog. */
export function parseEventList(raw: string): string[] {
  const events = new Set<string>();
  for (const token of raw.split(/[\s,]+/)) {
    const event = token.trim();
    if (event.length === 0) continue;
    events.add(event);
  }
  return [...events];
}

/** A compact label for the subscribed events column. */
export function eventsLabel(events: readonly string[]): string {
  if (events.length === 0) return "No events";
  if (events.includes("*")) return "All events";
  if (events.length === 1) return events[0] ?? "";
  return `${events[0]} +${events.length - 1}`;
}

/**
 * Endpoint URLs must be absolute http(s) URLs. https is required outside
 * local development because the API rejects private destinations.
 */
export function webhookUrlError(raw: string, allowHttp = false): string | null {
  const value = raw.trim();
  if (value.length === 0) return "Enter the URL that should receive events.";
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    return "Enter a complete URL, for example https://example.com/hooks.";
  }
  if (parsed.protocol !== "https:" && !(allowHttp && parsed.protocol === "http:")) {
    return "Use an https URL.";
  }
  return null;
}
