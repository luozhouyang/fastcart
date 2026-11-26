import type {
  AddStoreLocaleRequest,
  StoreLocale,
  UpdateStoreLocaleRequest,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/**
 * Store display locales.
 *
 * The default locale is the fallback for translated content; the API rejects
 * removing it or adding a second default.
 */
async function localeRequest<T>(storeId: string, path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("x-store-id", storeId);
  const csrf = readCookie("fc_csrf");
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/platform${path}`, {
    ...init,
    credentials: "include",
    headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(apiErrorMessage(body, `Locale request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists every locale enabled for the store, default first. */
export function listStoreLocales(storeId: string, signal?: AbortSignal): Promise<StoreLocale[]> {
  return localeRequest<StoreLocale[]>(
    storeId,
    `/stores/${encodeURIComponent(storeId)}/locales`,
    signal ? { signal } : {},
  );
}

/** Adds a locale, normalizing `fr_fr` to `fr-FR` server-side. */
export function addStoreLocale(
  storeId: string,
  input: AddStoreLocaleRequest,
  signal?: AbortSignal,
): Promise<StoreLocale> {
  return localeRequest<StoreLocale>(storeId, `/stores/${encodeURIComponent(storeId)}/locales`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Updates the label or direction of one locale. */
export function updateStoreLocale(
  storeId: string,
  locale: string,
  input: UpdateStoreLocaleRequest,
  signal?: AbortSignal,
): Promise<StoreLocale> {
  return localeRequest<StoreLocale>(
    storeId,
    `/stores/${encodeURIComponent(storeId)}/locales/${encodeURIComponent(locale)}`,
    {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      ...(signal ? { signal } : {}),
    },
  );
}

/** Removes a locale. The default locale cannot be removed. */
export function removeStoreLocale(
  storeId: string,
  locale: string,
  signal?: AbortSignal,
): Promise<void> {
  return localeRequest<void>(
    storeId,
    `/stores/${encodeURIComponent(storeId)}/locales/${encodeURIComponent(locale)}`,
    { method: "DELETE", ...(signal ? { signal } : {}) },
  );
}

/** Promotes one locale to the store default in a single transaction. */
export function setDefaultStoreLocale(
  storeId: string,
  locale: string,
  signal?: AbortSignal,
): Promise<StoreLocale> {
  return localeRequest<StoreLocale>(
    storeId,
    `/stores/${encodeURIComponent(storeId)}/locales/${encodeURIComponent(locale)}/default`,
    { method: "POST", ...(signal ? { signal } : {}) },
  );
}

/** Validates a BCP-47 locale tag before a request is sent. */
export function localeInputError(raw: string): string | null {
  const value = raw.trim().replace(/_/g, "-");
  if (value.length === 0) return "Enter a locale such as fr-FR or pt-BR.";
  if (value.length > 35) return "Locales cannot be longer than 35 characters.";
  if (!/^[A-Za-z]{2,8}(?:-[A-Za-z0-9]{1,8})*$/.test(value)) {
    return "Use a BCP-47 tag such as fr-FR, pt-BR or zh-Hant-TW.";
  }
  return null;
}
