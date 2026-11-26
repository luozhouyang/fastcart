import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiToken, Membership } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  apiTokenFormError,
  createApiToken,
  hasRole,
  isTokenActive,
  listApiTokens,
  listRoles,
  membershipRoleError,
  normalizeScopes,
  revokeApiToken,
  roleNames,
  tokenState,
} from "@/lib/platform-team";

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

function membership(overrides: Partial<Membership> = {}): Membership {
  return {
    id: "m1",
    user_id: "user_1",
    store_id: "store-1",
    status: "active",
    roles: ["admin"],
    created_at: 1,
    updated_at: 2,
    ...overrides,
  };
}

function token(overrides: Partial<ApiToken> = {}): ApiToken {
  return {
    id: "token_1",
    user_id: "user_1",
    store_id: "store-1",
    name: "CI",
    prefix: "fc_abcdefgh",
    scopes: ["products.read"],
    expires_at: 2_000_000_000,
    revoked_at: null,
    last_used_at: null,
    created_at: 1,
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

describe("platform team client", () => {
  it("lists roles with store and csrf headers", async () => {
    const roles = [{ name: "owner", description: "Full control of the store" }];
    const mock = vi.fn(async () => jsonResponse(roles));
    vi.stubGlobal("fetch", mock);

    await expect(listRoles("store-1")).resolves.toEqual(roles);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/platform/roles`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("lists tokens", async () => {
    const mock = vi.fn(async () => jsonResponse([token()]));
    vi.stubGlobal("fetch", mock);

    await listApiTokens("store-1");

    expect(requestAt(mock).url).toBe(`${API_BASE_URL}/api/v1/platform/api-tokens`);
  });

  it("creates a token with store scope and scopes", async () => {
    const mock = vi.fn(async () => jsonResponse({ token: "fc_secret", record: token() }, 201));
    vi.stubGlobal("fetch", mock);

    const created = await createApiToken("store-1", {
      name: "CI",
      store_id: "store-1",
      scopes: ["products.read"],
      ttl_seconds: 3600,
    });
    expect(created.token).toBe("fc_secret");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/platform/api-tokens`);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({
      name: "CI",
      store_id: "store-1",
      scopes: ["products.read"],
      ttl_seconds: 3600,
    });
  });

  it("revokes a token by encoded id", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await revokeApiToken("store-1", "token/1");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/platform/api-tokens/token%2F1`);
    expect(init.method).toBe("DELETE");
  });

  it("surfaces the API error message", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { code: "forbidden", message: "scope not granted" } }, 403),
    );
    vi.stubGlobal("fetch", mock);

    await expect(
      createApiToken("store-1", { name: "CI", store_id: "store-1", scopes: ["root"] }),
    ).rejects.toThrow("scope not granted");
  });
});

describe("team helpers", () => {
  it("collects sorted unique role names", () => {
    expect(
      roleNames([
        membership({ roles: ["admin", "owner"] }),
        membership({ roles: ["owner", "support-agent"] }),
      ]),
    ).toEqual(["admin", "owner", "support-agent"]);
  });

  it("checks role membership", () => {
    expect(hasRole({ roles: ["owner"] }, "owner")).toBe(true);
    expect(hasRole({ roles: ["owner"] }, "admin")).toBe(false);
  });

  it("requires at least one distinct role", () => {
    expect(membershipRoleError([])).toMatch(/at least one/);
    expect(membershipRoleError(["admin", "admin"])).toMatch(/duplicate/);
    expect(membershipRoleError(["admin"])).toBeNull();
  });

  it("labels token state from revocation and expiry", () => {
    const now = 1_000;
    expect(tokenState({ revoked_at: 5, expires_at: null }, now)).toEqual({
      label: "Revoked",
      tone: "destructive",
    });
    expect(tokenState({ revoked_at: null, expires_at: 500 }, now)).toEqual({
      label: "Expired",
      tone: "warning",
    });
    expect(tokenState({ revoked_at: null, expires_at: 5_000 }, now)).toEqual({
      label: "Active",
      tone: "brand",
    });
    expect(isTokenActive({ revoked_at: null, expires_at: null }, now)).toBe(true);
  });

  it("normalizes scopes", () => {
    expect(normalizeScopes([" products.read ", "orders.read", "products.read", ""])).toEqual([
      "orders.read",
      "products.read",
    ]);
  });

  it("validates the token form", () => {
    expect(apiTokenFormError({ name: "", scopes: ["products.read"] })).toMatch(/name/);
    expect(apiTokenFormError({ name: "CI", scopes: [] })).toMatch(/scope/);
    expect(apiTokenFormError({ name: "a".repeat(121), scopes: ["products.read"] })).toMatch(/120/);
    expect(apiTokenFormError({ name: "CI", scopes: ["products.read"] })).toBeNull();
  });
});
