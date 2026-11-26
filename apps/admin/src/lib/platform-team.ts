import type { ApiToken, CreatedApiToken, Membership, Role } from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** Body accepted by `POST /api/v1/platform/api-tokens`. */
export interface CreateApiTokenInput {
  name: string;
  store_id: string;
  scopes: string[];
  ttl_seconds?: number;
}

/** Token lifetimes offered by the creation dialog, in seconds. */
export const TOKEN_TTL_OPTIONS: ReadonlyArray<{ seconds: number; label: string }> = [
  { seconds: 30 * 24 * 60 * 60, label: "30 days" },
  { seconds: 90 * 24 * 60 * 60, label: "90 days" },
  { seconds: 365 * 24 * 60 * 60, label: "1 year" },
];

/**
 * Membership and role updates.
 *
 * The API exposes memberships as `GET`/`POST /stores/{id}/memberships` plus
 * `DELETE /stores/{id}/memberships/{user_id}`; there is no PATCH method, so
 * changing roles is a POST upsert with the full role list. The SDK already
 * wraps those three calls and the settings page uses them directly.
 */
async function platformRequest<T>(
  storeId: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
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
    throw new Error(apiErrorMessage(body, `Platform request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists the system roles that can be assigned to a store member. */
export function listRoles(storeId: string, signal?: AbortSignal): Promise<Role[]> {
  return platformRequest<Role[]>(storeId, "/roles", signal ? { signal } : {});
}

/** Lists the signed-in user's API tokens. Secrets are never returned here. */
export function listApiTokens(storeId: string, signal?: AbortSignal): Promise<ApiToken[]> {
  return platformRequest<ApiToken[]>(storeId, "/api-tokens", signal ? { signal } : {});
}

/** Creates a store-scoped token; the plaintext secret is returned exactly once. */
export function createApiToken(
  storeId: string,
  input: CreateApiTokenInput,
  signal?: AbortSignal,
): Promise<CreatedApiToken> {
  return platformRequest<CreatedApiToken>(storeId, "/api-tokens", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Revokes one of the signed-in user's tokens. */
export function revokeApiToken(
  storeId: string,
  tokenId: string,
  signal?: AbortSignal,
): Promise<void> {
  return platformRequest<void>(storeId, `/api-tokens/${encodeURIComponent(tokenId)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/** Sorted unique role names, for chips and select options. */
export function roleNames(memberships: readonly Membership[]): string[] {
  return [...new Set(memberships.flatMap((membership) => membership.roles))].sort();
}

/** True when the membership belongs to the given role name. */
export function hasRole(membership: Pick<Membership, "roles">, role: string): boolean {
  return membership.roles.includes(role);
}

/**
 * Validates a member's role selection.
 *
 * The API rejects an empty role list, so the UI catches it before a request.
 */
export function membershipRoleError(roles: readonly string[]): string | null {
  if (roles.length === 0) return "Choose at least one role.";
  if (new Set(roles).size !== roles.length) return "Remove the duplicate role.";
  return null;
}

/** Human-readable token state used by the table. */
export function tokenState(
  token: Pick<ApiToken, "revoked_at" | "expires_at">,
  nowSeconds = Math.floor(Date.now() / 1000),
): { label: string; tone: "brand" | "secondary" | "warning" | "destructive" } {
  if (token.revoked_at !== null) return { label: "Revoked", tone: "destructive" };
  if (token.expires_at !== null && token.expires_at <= nowSeconds) {
    return { label: "Expired", tone: "warning" };
  }
  return { label: "Active", tone: "brand" };
}

/** True when the token can still authenticate requests. */
export function isTokenActive(
  token: Pick<ApiToken, "revoked_at" | "expires_at">,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  return tokenState(token, nowSeconds).label === "Active";
}

/** Scopes that are likely useful as a starting point, grouped for the dialog. */
export const SCOPE_PRESETS: ReadonlyArray<{ id: string; label: string; scopes: string[] }> = [
  {
    id: "read-only",
    label: "Read-only",
    scopes: [
      "products.read",
      "orders.read",
      "customers.read",
      "inventory.read",
      "settings.read",
      "themes.read",
    ],
  },
  {
    id: "catalog",
    label: "Catalog and inventory",
    scopes: ["products.read", "products.write", "inventory.read", "inventory.write"],
  },
  {
    id: "orders",
    label: "Orders and customers",
    scopes: ["orders.read", "orders.write", "customers.read", "customers.write"],
  },
];

/** Sorted, de-duplicated scope list. */
export function normalizeScopes(scopes: readonly string[]): string[] {
  return [
    ...new Set(scopes.map((scope) => scope.trim()).filter((scope) => scope.length > 0)),
  ].sort();
}

/** Validation shared by the token dialog. */
export function apiTokenFormError(input: {
  name: string;
  scopes: readonly string[];
}): string | null {
  const name = input.name.trim();
  if (name.length === 0) return "Give the token a name.";
  if (name.length > 120) return "Token names cannot be longer than 120 characters.";
  if (input.scopes.length === 0) return "Select at least one scope.";
  return null;
}
