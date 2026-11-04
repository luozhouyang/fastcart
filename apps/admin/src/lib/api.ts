import { createAdminClient, type AdminClient } from "@fastcart/admin-sdk";
import { ApiError } from "@fastcart/api-client";
import type { PageResult, PaginationQuery, Store } from "@fastcart/shared-types";

/**
 * The API origin used by the admin. In development the Rust server runs on
 * 8080; a production build talks to the same origin that served the app.
 */
export const API_BASE_URL =
  (import.meta.env.VITE_FASTCART_API_URL as string | undefined) ??
  (import.meta.env.DEV ? "http://localhost:8080" : "");

export const DEMO_STORAGE_KEY = "fastcart-demo";
export const STORE_STORAGE_KEY = "fastcart-store";

export function readCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  const prefix = `${name}=`;
  for (const part of document.cookie.split(";")) {
    const entry = part.trim();
    if (entry.startsWith(prefix)) {
      try {
        return decodeURIComponent(entry.slice(prefix.length));
      } catch {
        return entry.slice(prefix.length);
      }
    }
  }
  return null;
}

function csrfToken(): string | null {
  return readCookie("fc_csrf");
}

/**
 * Creates an admin client for a store. The bootstrap client uses an empty
 * store header so `/auth/me` and `/stores` resolve the user's own stores
 * instead of authorizing against a specific one.
 */
export function createAdmin(storeId = ""): AdminClient {
  return createAdminClient({
    baseUrl: API_BASE_URL,
    storeId,
    credentials: "include",
    csrfToken,
  });
}

/** Loads every store the signed-in user can administer. */
export async function fetchStores(client: AdminClient, signal?: AbortSignal): Promise<Store[]> {
  const stores: Store[] = [];
  let offset = 0;
  for (;;) {
    const page = await client.stores.list({ limit: 100, offset }, signal ? { signal } : undefined);
    stores.push(...page.data);
    offset += page.data.length;
    if (page.data.length === 0 || offset >= page.pagination.total) break;
  }
  return stores;
}

/** Reads every record of a paginated endpoint, capped for safety. */
export async function fetchAll<T>(
  fetchPage: (query: PaginationQuery) => Promise<PageResult<T>>,
  max = 1000,
): Promise<T[]> {
  const records: T[] = [];
  let offset = 0;
  while (records.length < max) {
    const remaining = max - records.length;
    const page = await fetchPage({ limit: Math.min(100, Math.max(1, remaining)), offset });
    records.push(...page.data);
    offset += page.data.length;
    if (page.data.length === 0 || offset >= page.pagination.total) break;
  }
  return records;
}

/** Turns SDK failures into copy the login and page surfaces can show. */
export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 401) return "Your session has expired. Sign in again.";
    if (error.status === 403) return "You do not have permission to do that.";
    if (error.status === 429) return "Too many attempts. Wait a moment and try again.";
    if (error.status >= 500) return "The server could not complete the request.";
    return error.message;
  }
  if (error instanceof DOMException && error.name === "AbortError") {
    return "Request canceled.";
  }
  if (error instanceof TypeError) {
    return `Cannot reach the Fastcart API${API_BASE_URL ? ` at ${API_BASE_URL}` : ""}.`;
  }
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}
