import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CustomerGroup } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  addCustomerToGroup,
  createCustomerGroup,
  customerDisplayName,
  hasCustomerGroup,
  listCustomerGroups,
  listGroupsForCustomer,
  removeCustomerFromGroup,
  searchCustomers,
  sortCustomerGroups,
} from "@/lib/customer-groups";

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

function requestAt(
  mock: { mock: { calls: unknown[][] } },
  index = 0,
): { url: string; init: RequestInit } {
  const call = mock.mock.calls[index] as [string, RequestInit] | undefined;
  if (!call) throw new Error("fetch was not called");
  return { url: call[0], init: call[1] };
}

function group(overrides: Partial<CustomerGroup> = {}): CustomerGroup {
  return {
    id: "cgroup_1",
    name: "VIP",
    metadata: {},
    created_at: 1,
    updated_at: 2,
    ...overrides,
  };
}

beforeEach(() => {
  document.cookie = "fc_csrf=csrf-token";
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "fc_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT";
});

describe("customer group client", () => {
  it("lists groups with store and csrf headers", async () => {
    const mock = vi.fn(async () => jsonResponse([group()]));
    vi.stubGlobal("fetch", mock);

    await expect(listCustomerGroups("store-1")).resolves.toEqual([group()]);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/customer-groups`);
    expect(init.method).toBeUndefined();
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("posts the group name as JSON", async () => {
    const mock = vi.fn(async () => jsonResponse(group({ name: "Wholesale" }), 201));
    vi.stubGlobal("fetch", mock);

    await createCustomerGroup("store-1", { name: "Wholesale" });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/customer-groups`);
    expect(init.method).toBe("POST");
    expect((init.headers as Headers).get("content-type")).toBe("application/json");
    expect(JSON.parse(init.body as string)).toEqual({ name: "Wholesale" });
  });

  it("lists the groups of one customer", async () => {
    const mock = vi.fn(async () => jsonResponse([group()]));
    vi.stubGlobal("fetch", mock);

    await listGroupsForCustomer("store-1", "cust/1");

    const { url } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/customers/cust%2F1/groups`);
  });

  it("adds a customer to a group with the customer id body", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(addCustomerToGroup("store-1", "cgroup_1", "cust_9")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/customer-groups/cgroup_1/customers`);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ customer_id: "cust_9" });
  });

  it("removes a customer from a group", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await removeCustomerFromGroup("store-1", "cgroup_1", "cust_9");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/customer-groups/cgroup_1/customers/cust_9`);
    expect(init.method).toBe("DELETE");
  });

  it("searches customers with a query and a bounded page size", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({
        data: [
          {
            id: "cust_1",
            email: "ava@example.com",
            first_name: "Ava",
            last_name: "Whitfield",
            phone: null,
            has_account: true,
            status: "active",
            email_verified_at: null,
            metadata: {},
            created_at: 1,
            updated_at: 2,
            deleted_at: null,
          },
        ],
        pagination: { limit: 8, offset: 0, total: 1 },
      }),
    );
    vi.stubGlobal("fetch", mock);

    const customers = await searchCustomers("store-1", "  ava  ");
    expect(customers).toHaveLength(1);

    const { url } = requestAt(mock);
    expect(url).toContain("/api/v1/admin/customers?");
    expect(url).toContain("query=ava");
    expect(url).toContain("limit=8");
  });

  it("surfaces the API error message", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { code: "conflict", message: "group already exists" } }, 409),
    );
    vi.stubGlobal("fetch", mock);

    await expect(createCustomerGroup("store-1", { name: "VIP" })).rejects.toThrow(
      "group already exists",
    );
  });

  it("forwards an abort signal", async () => {
    const mock = vi.fn(async () => jsonResponse([]));
    vi.stubGlobal("fetch", mock);
    const controller = new AbortController();

    await listCustomerGroups("store-1", controller.signal);

    const { init } = requestAt(mock);
    expect(init.signal).toBe(controller.signal);
  });
});

describe("customer group helpers", () => {
  it("sorts groups by name without mutating the input", () => {
    const input = [group({ id: "b", name: "Wholesale" }), group({ id: "a", name: "VIP" })];
    const sorted = sortCustomerGroups(input);

    expect(sorted.map((entry) => entry.name)).toEqual(["VIP", "Wholesale"]);
    expect(input.map((entry) => entry.name)).toEqual(["Wholesale", "VIP"]);
  });

  it("detects group membership", () => {
    expect(hasCustomerGroup([group({ id: "cgroup_1" })], "cgroup_1")).toBe(true);
    expect(hasCustomerGroup([group({ id: "cgroup_1" })], "cgroup_2")).toBe(false);
  });

  it("prefers the customer name and falls back to the email", () => {
    expect(
      customerDisplayName({ first_name: "Ava", last_name: "Whitfield", email: "ava@example.com" }),
    ).toBe("Ava Whitfield");
    expect(
      customerDisplayName({ first_name: null, last_name: null, email: "guest@example.com" }),
    ).toBe("guest@example.com");
  });
});
