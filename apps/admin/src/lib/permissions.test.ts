import { describe, expect, it } from "vitest";

import {
  PERMISSIONS,
  canAll,
  canAny,
  missingPermissionHint,
  type Permission,
} from "@/lib/permissions";

const granted = (permissions: string[]) => (permission: Permission) =>
  permissions.includes("*") || permissions.includes(permission);

describe("permission constants", () => {
  it("matches the permission strings enforced by the API", () => {
    expect(PERMISSIONS.products.read).toBe("products.read");
    expect(PERMISSIONS.products.write).toBe("products.write");
    expect(PERMISSIONS.orders.cancel).toBe("orders.cancel");
    expect(PERMISSIONS.customers.write).toBe("customers.write");
    expect(PERMISSIONS.inventory.write).toBe("inventory.write");
    expect(PERMISSIONS.promotions.read).toBe("promotions.read");
    expect(PERMISSIONS.settings.write).toBe("settings.write");
    expect(PERMISSIONS.themes.read).toBe("themes.read");
  });
});

describe("canAny", () => {
  const can = granted(["products.write"]);

  it("returns true when one of the permissions is granted", () => {
    expect(canAny(can, [PERMISSIONS.products.read, PERMISSIONS.products.write])).toBe(true);
  });

  it("returns false when none of the permissions are granted", () => {
    expect(canAny(can, [PERMISSIONS.orders.write])).toBe(false);
  });

  it("returns false for an empty requirement", () => {
    expect(canAny(can, [])).toBe(false);
  });
});

describe("canAll", () => {
  const can = granted(["products.read", "products.write"]);

  it("returns true when every permission is granted", () => {
    expect(canAll(can, [PERMISSIONS.products.read, PERMISSIONS.products.write])).toBe(true);
  });

  it("returns false when one permission is missing", () => {
    expect(canAll(can, [PERMISSIONS.products.read, PERMISSIONS.orders.write])).toBe(false);
  });

  it("returns true for an empty requirement", () => {
    expect(canAll(can, [])).toBe(true);
  });

  it("honours the wildcard grant", () => {
    expect(canAll(granted(["*"]), [PERMISSIONS.products.write, PERMISSIONS.settings.write])).toBe(
      true,
    );
  });
});

describe("missingPermissionHint", () => {
  it("names the missing permission", () => {
    expect(missingPermissionHint(PERMISSIONS.products.write)).toContain("products.write");
  });
});
