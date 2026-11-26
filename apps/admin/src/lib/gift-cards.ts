import type {
  CreateGiftCardRequest,
  GiftCard,
  GiftCardStatus,
  PageResult,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** One page of gift cards. */
export type GiftCardPage = PageResult<GiftCard>;

async function giftCardRequest<T>(
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
    throw new Error(apiErrorMessage(body, `Gift card request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists one page of gift cards, newest first. */
export function listGiftCards(
  storeId: string,
  query: { limit?: number; offset?: number } = {},
  signal?: AbortSignal,
): Promise<GiftCardPage> {
  const params = new URLSearchParams({
    limit: String(query.limit ?? 25),
    offset: String(query.offset ?? 0),
  });
  return giftCardRequest<GiftCardPage>(
    storeId,
    `/gift-cards?${params.toString()}`,
    signal ? { signal } : {},
  );
}

/** Issues a card. The API generates a random code when `code` is omitted. */
export function issueGiftCard(
  storeId: string,
  input: CreateGiftCardRequest,
  signal?: AbortSignal,
): Promise<GiftCard> {
  return giftCardRequest<GiftCard>(storeId, "/gift-cards", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Enables or disables a card. Both calls are idempotent. */
export function setGiftCardEnabled(
  storeId: string,
  giftCardId: string,
  enabled: boolean,
  signal?: AbortSignal,
): Promise<GiftCard> {
  return giftCardRequest<GiftCard>(
    storeId,
    `/gift-cards/${encodeURIComponent(giftCardId)}/${enabled ? "enable" : "disable"}`,
    { method: "POST", ...(signal ? { signal } : {}) },
  );
}

/** Share of the initial balance still available, clamped to `[0, 1]`. */
export function remainingFraction(card: Pick<GiftCard, "initial_amount_minor" | "balance_minor">) {
  if (card.initial_amount_minor <= 0) return 0;
  return Math.min(1, Math.max(0, card.balance_minor / card.initial_amount_minor));
}

/** Merchant-facing status label. */
export function giftCardStatusLabel(status: GiftCardStatus): string {
  return status === "active" ? "Active" : "Disabled";
}

/** Validates the issue form before a request is sent. */
export function giftCardInputError(input: {
  amount: string;
  currency: string;
  code: string;
}): string | null {
  if (!/^[A-Za-z]{3}$/.test(input.currency.trim())) {
    return "Enter a three-letter currency code such as USD.";
  }
  const amount = Number(input.amount);
  if (!Number.isFinite(amount) || amount <= 0) {
    return "Enter an amount greater than zero.";
  }
  if (Math.round(amount * 100) <= 0) {
    return "Enter an amount of at least one cent.";
  }
  if (input.code.trim().length > 64) {
    return "Codes cannot be longer than 64 characters.";
  }
  return null;
}
