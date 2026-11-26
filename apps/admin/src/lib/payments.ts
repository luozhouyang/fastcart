import type {
  CreatePaymentRefundRequest,
  FailPaymentSessionRequest,
  PageResult,
  Payment,
  PaymentRefund,
  PaymentSession,
  PaymentStatus,
  Timestamp,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/**
 * Payment statuses the API may return. `shared-types` has not caught up with
 * `partially_captured`, which the payment module emits for partial captures.
 */
export type AdminPaymentStatus = PaymentStatus | "partially_captured";

/** A payment row with the full status set the admin API returns. */
export interface AdminPayment extends Omit<Payment, "status"> {
  status: AdminPaymentStatus;
}

/** A payment session row with the full status set the admin API returns. */
export interface AdminPaymentSession extends Omit<PaymentSession, "status"> {
  status: AdminPaymentStatus;
}

/** Query accepted by the payment collection. */
export interface PaymentQuery {
  order_id?: string;
  status?: AdminPaymentStatus;
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

async function paymentRequest<T>(
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
    throw new Error(apiErrorMessage(body, `Payment request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function listPayments(storeId: string, query: PaymentQuery = {}, signal?: AbortSignal) {
  const path = `/payments${queryString({
    order_id: query.order_id,
    status: query.status,
    limit: query.limit,
    offset: query.offset,
  })}`;
  return paymentRequest<PageResult<AdminPayment>>(storeId, path, signal ? { signal } : {});
}

export function getPayment(storeId: string, paymentId: string, signal?: AbortSignal) {
  return paymentRequest<AdminPayment>(storeId, `/payments/${encodeURIComponent(paymentId)}`, {
    ...(signal ? { signal } : {}),
  });
}

export function listPaymentRefunds(storeId: string, paymentId: string, signal?: AbortSignal) {
  return paymentRequest<PaymentRefund[]>(
    storeId,
    `/payments/${encodeURIComponent(paymentId)}/refunds`,
    signal ? { signal } : {},
  );
}

export function createPaymentRefund(
  storeId: string,
  paymentId: string,
  body: CreatePaymentRefundRequest,
  signal?: AbortSignal,
) {
  return paymentRequest<PaymentRefund>(
    storeId,
    `/payments/${encodeURIComponent(paymentId)}/refunds`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      ...(signal ? { signal } : {}),
    },
  );
}

export function capturePaymentSession(storeId: string, sessionId: string, signal?: AbortSignal) {
  return paymentRequest<AdminPayment>(
    storeId,
    `/payment-sessions/${encodeURIComponent(sessionId)}/capture`,
    { method: "POST", ...(signal ? { signal } : {}) },
  );
}

export function cancelPaymentSession(storeId: string, sessionId: string, signal?: AbortSignal) {
  return paymentRequest<AdminPaymentSession>(
    storeId,
    `/payment-sessions/${encodeURIComponent(sessionId)}/cancel`,
    { method: "POST", ...(signal ? { signal } : {}) },
  );
}

export function failPaymentSession(
  storeId: string,
  sessionId: string,
  body: FailPaymentSessionRequest = {},
  signal?: AbortSignal,
) {
  return paymentRequest<AdminPaymentSession>(
    storeId,
    `/payment-sessions/${encodeURIComponent(sessionId)}/fail`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      ...(signal ? { signal } : {}),
    },
  );
}

export const PAYMENT_STATUSES: readonly AdminPaymentStatus[] = [
  "pending",
  "requires_action",
  "authorized",
  "partially_captured",
  "captured",
  "partially_refunded",
  "refunded",
  "failed",
  "canceled",
];

export const PAYMENT_STATUS_LABELS: Record<AdminPaymentStatus, string> = {
  pending: "Pending",
  requires_action: "Requires action",
  authorized: "Authorized",
  partially_captured: "Partially captured",
  captured: "Captured",
  partially_refunded: "Partially refunded",
  refunded: "Refunded",
  failed: "Failed",
  canceled: "Canceled",
};

export type PaymentStatusTone = "brand" | "warning" | "info" | "destructive" | "secondary";

/** Status filter options rendered by the payments page. */
export function paymentFilterOptions(): Array<{
  value: AdminPaymentStatus | "all";
  label: string;
}> {
  return [
    { value: "all", label: "All" },
    ...PAYMENT_STATUSES.map((status) => ({
      value: status,
      label: PAYMENT_STATUS_LABELS[status],
    })),
  ];
}

/** Payment method label shown in the table; providers stay lowercase. */
export function paymentMethodLabel(payment: Pick<AdminPayment, "provider">): string {
  const provider = payment.provider.trim();
  if (provider.length === 0) return "Manual";
  return provider;
}

export const PAYMENT_STATUS_TONES: Record<AdminPaymentStatus, PaymentStatusTone> = {
  pending: "secondary",
  requires_action: "warning",
  authorized: "info",
  partially_captured: "warning",
  captured: "brand",
  partially_refunded: "warning",
  refunded: "destructive",
  failed: "destructive",
  canceled: "secondary",
};

/** True when the status is known to the table and can be labelled. */
export function paymentStatusLabel(status: string): string {
  return PAYMENT_STATUS_LABELS[status as AdminPaymentStatus] ?? status;
}

/** Badge tone for a status, defaulting to secondary for unknown values. */
export function paymentStatusTone(status: string): PaymentStatusTone {
  return PAYMENT_STATUS_TONES[status as AdminPaymentStatus] ?? "secondary";
}

/**
 * The payment session that produced a payment, when the API exposes one.
 *
 * Payments carry the session identifier in their metadata for some providers;
 * session actions are only offered when it is present.
 */
export function paymentSessionId(payment: Pick<AdminPayment, "metadata">): string | null {
  for (const key of ["session_id", "payment_session_id"]) {
    const value = payment.metadata[key];
    if (typeof value === "string" && value.length > 0) return value;
  }
  return null;
}

/** Sum of refunds that were not rejected by the provider. */
export function refundedMinor(refunds: readonly PaymentRefund[]): number {
  return refunds
    .filter((refund) => refund.status !== "failed")
    .reduce((total, refund) => total + refund.amount, 0);
}

/** True when the payment holds captured money the API will refund. */
export function isRefundableStatus(status: AdminPaymentStatus): boolean {
  return (
    status === "captured" || status === "partially_captured" || status === "partially_refunded"
  );
}

/** Amount still refundable after the recorded refunds; 0 for other statuses. */
export function refundableMinor(
  payment: Pick<AdminPayment, "amount" | "status">,
  refunds: readonly PaymentRefund[],
): number {
  if (!isRefundableStatus(payment.status)) return 0;
  return Math.max(0, payment.amount - refundedMinor(refunds));
}

/** True when a payment can receive a new refund. */
export function canRefundPayment(
  payment: Pick<AdminPayment, "amount" | "status">,
  refunds: readonly PaymentRefund[],
): boolean {
  return refundableMinor(payment, refunds) > 0;
}

/** Label for the last activity timestamp of a payment. */
export function paymentActivityAt(
  payment: Pick<AdminPayment, "captured_at" | "updated_at">,
): Timestamp {
  return payment.captured_at ?? payment.updated_at;
}
