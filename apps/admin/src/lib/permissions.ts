/**
 * Permission strings enforced by the Fastcart admin API.
 *
 * Keep these in sync with `crates/api/src/routes/admin_*.rs`. The API is the
 * source of truth: the admin UI only uses these names to hide controls the
 * signed-in account cannot use.
 */
export const PERMISSIONS = {
  products: {
    read: "products.read",
    write: "products.write",
  },
  orders: {
    read: "orders.read",
    write: "orders.write",
    cancel: "orders.cancel",
  },
  customers: {
    read: "customers.read",
    write: "customers.write",
  },
  inventory: {
    read: "inventory.read",
    write: "inventory.write",
  },
  promotions: {
    read: "promotions.read",
    write: "promotions.write",
  },
  settings: {
    read: "settings.read",
    write: "settings.write",
  },
  themes: {
    read: "themes.read",
    write: "themes.write",
  },
  payments: {
    read: "payments.read",
    write: "payments.write",
  },
} as const;

type PermissionTree = typeof PERMISSIONS;

/** Every permission string understood by the admin UI. */
export type Permission = {
  [Group in keyof PermissionTree]: PermissionTree[Group][keyof PermissionTree[Group]];
}[keyof PermissionTree];

export type PermissionChecker = (permission: Permission) => boolean;

/** True when the checker grants at least one of `permissions`. */
export function canAny(
  hasPermission: PermissionChecker,
  permissions: readonly Permission[],
): boolean {
  return permissions.some((permission) => hasPermission(permission));
}

/** True when the checker grants every permission in `permissions`. */
export function canAll(
  hasPermission: PermissionChecker,
  permissions: readonly Permission[],
): boolean {
  return permissions.every((permission) => hasPermission(permission));
}

/** Builds the tooltip shown on controls disabled for a missing permission. */
export function missingPermissionHint(permission: Permission): string {
  return `Your account does not have the "${permission}" permission.`;
}
