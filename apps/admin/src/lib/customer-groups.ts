import type { Customer, CustomerGroup, Metadata } from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** Envelope returned by the paginated admin endpoints. */
export interface CustomerGroupPage<T> {
  data: T[];
  pagination: {
    limit: number;
    offset: number;
    total: number;
  };
}

/** Body accepted by `POST /api/v1/admin/customer-groups`. */
export interface CreateCustomerGroupInput {
  name: string;
  metadata?: Metadata;
}

/** Largest page the customer search requests when suggesting members. */
export const MEMBER_SEARCH_LIMIT = 8;

async function groupRequest<T>(storeId: string, path: string, init: RequestInit = {}): Promise<T> {
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
    throw new Error(apiErrorMessage(body, `Customer group request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists every customer group in name order, as the API already sorts them. */
export function listCustomerGroups(
  storeId: string,
  signal?: AbortSignal,
): Promise<CustomerGroup[]> {
  return groupRequest<CustomerGroup[]>(storeId, "/customer-groups", signal ? { signal } : {});
}

/** Creates a group. Names are unique within the store. */
export function createCustomerGroup(
  storeId: string,
  input: CreateCustomerGroupInput,
  signal?: AbortSignal,
): Promise<CustomerGroup> {
  return groupRequest<CustomerGroup>(storeId, "/customer-groups", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Lists the groups one customer belongs to. */
export function listGroupsForCustomer(
  storeId: string,
  customerId: string,
  signal?: AbortSignal,
): Promise<CustomerGroup[]> {
  return groupRequest<CustomerGroup[]>(
    storeId,
    `/customers/${encodeURIComponent(customerId)}/groups`,
    signal ? { signal } : {},
  );
}

/** Adds a customer to a group. Assigning twice is a server-side no-op. */
export function addCustomerToGroup(
  storeId: string,
  groupId: string,
  customerId: string,
  signal?: AbortSignal,
): Promise<void> {
  return groupRequest<void>(storeId, `/customer-groups/${encodeURIComponent(groupId)}/customers`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ customer_id: customerId }),
    ...(signal ? { signal } : {}),
  });
}

/** Removes a customer from a group. */
export function removeCustomerFromGroup(
  storeId: string,
  groupId: string,
  customerId: string,
  signal?: AbortSignal,
): Promise<void> {
  return groupRequest<void>(
    storeId,
    `/customer-groups/${encodeURIComponent(groupId)}/customers/${encodeURIComponent(customerId)}`,
    {
      method: "DELETE",
      ...(signal ? { signal } : {}),
    },
  );
}

/** Searches customers by email or name for the member picker. */
export async function searchCustomers(
  storeId: string,
  query: string,
  signal?: AbortSignal,
): Promise<Customer[]> {
  const search = new URLSearchParams();
  const trimmed = query.trim();
  if (trimmed.length > 0) search.set("query", trimmed);
  search.set("limit", String(MEMBER_SEARCH_LIMIT));
  const page = await groupRequest<CustomerGroupPage<Customer>>(
    storeId,
    `/customers?${search.toString()}`,
    signal ? { signal } : {},
  );
  return page.data;
}

/** Display name used by search results, falling back to the email address. */
export function customerDisplayName(
  customer: Pick<Customer, "first_name" | "last_name" | "email">,
): string {
  const name = [customer.first_name, customer.last_name]
    .map((part) => part?.trim() ?? "")
    .filter((part) => part.length > 0)
    .join(" ");
  return name.length > 0 ? name : customer.email;
}

/** Sorts groups by name, keeping the input untouched. */
export function sortCustomerGroups(groups: readonly CustomerGroup[]): CustomerGroup[] {
  return [...groups].sort(
    (left, right) => left.name.localeCompare(right.name) || left.id.localeCompare(right.id),
  );
}

/** True when the customer's group list contains the given group. */
export function hasCustomerGroup(groups: readonly CustomerGroup[], groupId: string): boolean {
  return groups.some((group) => group.id === groupId);
}
