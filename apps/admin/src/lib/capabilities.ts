import { API_BASE_URL, readCookie } from "@/lib/api";

/** JSON value accepted by capability configuration objects. */
export type JsonValue =
  boolean | number | string | null | JsonValue[] | { [key: string]: JsonValue };

/** JSON object shape used for capability configuration. */
export type JsonObject = { [key: string]: JsonValue };

/** One capability contribution declared by an enabled plugin. */
export interface CapabilityContribution {
  plugin_id: string;
  plugin_name: string;
  plugin_version: string;
  id: string;
  version: number;
  title: string;
  config_schema: JsonObject | null;
}

/** The provider bound to one capability for the current store. */
export interface CapabilityBinding {
  capability_id: string;
  version: number;
  plugin_id: string;
  config: JsonObject;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

/** Capability catalog and the store's current bindings. */
export interface CapabilitiesResponse {
  contributions: CapabilityContribution[];
  bindings: CapabilityBinding[];
}

/** Replacement payload for a capability binding. */
export interface BindCapabilityRequest {
  plugin_id: string;
  version: number;
  config?: JsonObject;
  enabled?: boolean;
}

/** A capability id together with its providers and current binding. */
export interface CapabilityGroup {
  id: string;
  title: string;
  contributions: CapabilityContribution[];
  binding: CapabilityBinding | null;
}

async function capabilityRequest<T>(
  storeId: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("x-store-id", storeId);
  const csrf = readCookie("fc_csrf");
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/admin/capabilities${path}`, {
    ...init,
    credentials: "include",
    headers,
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `Capability request failed (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function listCapabilities(storeId: string, signal?: AbortSignal) {
  return capabilityRequest<CapabilitiesResponse>(storeId, "", signal ? { signal } : {});
}

export function bindCapability(
  storeId: string,
  capabilityId: string,
  input: BindCapabilityRequest,
  signal?: AbortSignal,
) {
  return capabilityRequest<CapabilityBinding>(
    storeId,
    `/${encodeURIComponent(capabilityId)}/binding`,
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      ...(signal ? { signal } : {}),
    },
  );
}

export function unbindCapability(storeId: string, capabilityId: string, signal?: AbortSignal) {
  return capabilityRequest<void>(storeId, `/${encodeURIComponent(capabilityId)}/binding`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/** True when a parsed JSON value is a non-null, non-array object. */
export function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Parses the config editor text. The API only accepts an object for a
 * capability config, so arrays, primitives and malformed JSON are rejected
 * before a request is sent.
 */
export function parseConfigObject(text: string): {
  config: JsonObject | null;
  error: string | null;
} {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { config: null, error: "Config must be valid JSON." };
  }
  if (!isJsonObject(parsed)) {
    return { config: null, error: "Config must be a JSON object." };
  }
  return { config: parsed, error: null };
}

/**
 * Seeds an editor config from a contribution's JSON Schema. A top-level
 * `default` object wins; otherwise the declared property defaults are
 * collected so common schemas open with a useful starting point.
 */
export function defaultConfigForSchema(schema: JsonObject | null | undefined): JsonObject {
  if (!isJsonObject(schema)) return {};
  const declared = schema["default"];
  if (isJsonObject(declared)) return declared;
  const properties = schema["properties"];
  if (!isJsonObject(properties)) return {};
  const config: JsonObject = {};
  for (const [key, property] of Object.entries(properties)) {
    if (!isJsonObject(property)) continue;
    const fallback = property["default"];
    if (fallback !== undefined) config[key] = fallback;
  }
  return config;
}

/** Stable key for one provider offered by a capability id. */
export function providerKey(
  contribution: Pick<CapabilityContribution, "plugin_id" | "version">,
): string {
  return `${contribution.plugin_id}@${contribution.version}`;
}

/** True when the contribution is the provider recorded in the binding. */
export function isBoundProvider(
  binding: CapabilityBinding | null,
  contribution: CapabilityContribution,
): boolean {
  return (
    binding !== null &&
    binding.plugin_id === contribution.plugin_id &&
    binding.version === contribution.version
  );
}

/** The provider selected when the binding dialog opens. */
export function initialContribution(group: CapabilityGroup): CapabilityContribution | null {
  const bound = group.contributions.find((contribution) =>
    isBoundProvider(group.binding, contribution),
  );
  return bound ?? group.contributions[0] ?? null;
}

/**
 * Config shown when the dialog opens or the provider changes. A binding for
 * the selected provider wins so edits start from what the store already runs.
 */
export function initialConfig(
  group: CapabilityGroup,
  contribution: CapabilityContribution | null,
): JsonObject {
  if (group.binding && contribution && isBoundProvider(group.binding, contribution)) {
    return isJsonObject(group.binding.config) ? group.binding.config : {};
  }
  return defaultConfigForSchema(contribution?.config_schema);
}

/** Resolves a provider key back to its contribution. */
export function contributionForKey(
  group: CapabilityGroup | null,
  key: string,
): CapabilityContribution | null {
  if (!group) return null;
  return group.contributions.find((contribution) => providerKey(contribution) === key) ?? null;
}

/**
 * Groups contributions by capability id and attaches the store's binding.
 * Bindings whose provider is no longer enabled still appear so an admin can
 * remove or replace them.
 */
export function groupCapabilities(response: CapabilitiesResponse): CapabilityGroup[] {
  const groups = new Map<string, CapabilityGroup>();
  for (const contribution of response.contributions) {
    let group = groups.get(contribution.id);
    if (!group) {
      group = {
        id: contribution.id,
        title: contribution.title,
        contributions: [],
        binding: null,
      };
      groups.set(contribution.id, group);
    }
    group.contributions.push(contribution);
  }
  for (const binding of response.bindings) {
    let group = groups.get(binding.capability_id);
    if (!group) {
      group = {
        id: binding.capability_id,
        title: binding.capability_id,
        contributions: [],
        binding: null,
      };
      groups.set(binding.capability_id, group);
    }
    group.binding = binding;
    const bound = group.contributions.find((contribution) =>
      isBoundProvider(binding, contribution),
    );
    if (bound) group.title = bound.title;
  }
  for (const group of groups.values()) {
    group.contributions.sort(
      (left, right) =>
        left.plugin_name.localeCompare(right.plugin_name) ||
        left.plugin_id.localeCompare(right.plugin_id) ||
        right.version - left.version,
    );
  }
  return [...groups.values()].sort(
    (left, right) => left.title.localeCompare(right.title) || left.id.localeCompare(right.id),
  );
}
