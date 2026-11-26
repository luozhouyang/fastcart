import { API_BASE_URL, apiErrorMessage } from "@/lib/api";
import type { AdminExtension, AdminExtensionsResponse } from "@/plugins/types";

export interface InstalledPlugin {
  id: string;
  name: string;
  version: string;
  digest: string;
  previous_digest: string | null;
  enabled: boolean;
  settings: Record<string, unknown>;
  settings_revision: number;
  granted_scopes: string[];
  installed_at: number;
  updated_at: number;
}

export interface PluginListResponse {
  plugins: InstalledPlugin[];
  artifacts: Array<{ digest: string; id: string; name: string; version: string }>;
}

export interface PluginSecretName {
  name: string;
  updated_at: number;
}

function csrfToken(): string | null {
  const match = document.cookie.match(/(?:^|; )fc_csrf=([^;]*)/);
  return match ? decodeURIComponent(match[1] ?? "") : null;
}

/** Sends a store-scoped request to the admin plugin API. */
export async function pluginRequest<T>(
  storeId: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("x-store-id", storeId);
  const csrf = csrfToken();
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/admin/plugins${path}`, {
    ...init,
    credentials: "include",
    headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(apiErrorMessage(body, `Plugin request failed (${response.status})`));
  }
  // Accepted and no-content endpoints acknowledge work without a JSON body.
  if (response.status === 202 || response.status === 204) return undefined as T;
  const text = await response.text();
  if (text.length === 0) return undefined as T;
  return JSON.parse(text) as T;
}

export async function fetchPluginExtensions(
  storeId: string,
  signal?: AbortSignal,
): Promise<AdminExtension[]> {
  const body = await pluginRequest<AdminExtensionsResponse>(
    storeId,
    "/extensions",
    signal ? { signal } : {},
  );
  return body.extensions;
}

export function listPlugins(storeId: string, signal?: AbortSignal) {
  return pluginRequest<PluginListResponse>(storeId, "", signal ? { signal } : {});
}

export function installPlugin(storeId: string, archive: File) {
  const body = new FormData();
  body.append("archive", archive);
  return pluginRequest<InstalledPlugin>(storeId, "/install", {
    method: "POST",
    body,
  });
}

export function enablePlugin(storeId: string, pluginId: string) {
  return pluginRequest<InstalledPlugin>(storeId, `/${pluginId}/enable`, { method: "POST" });
}

export function disablePlugin(storeId: string, pluginId: string) {
  return pluginRequest<InstalledPlugin>(storeId, `/${pluginId}/disable`, { method: "POST" });
}

export function deletePlugin(storeId: string, pluginId: string) {
  return pluginRequest<void>(storeId, `/${pluginId}`, { method: "DELETE" });
}

export function updatePluginSettings(storeId: string, pluginId: string, settings: unknown) {
  return pluginRequest<InstalledPlugin>(storeId, `/${pluginId}/settings`, {
    method: "PATCH",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ settings }),
  });
}

export function listPluginSecrets(
  storeId: string,
  pluginId: string,
  signal?: AbortSignal,
): Promise<PluginSecretName[]> {
  return pluginRequest<PluginSecretName[]>(
    storeId,
    `/${pluginId}/secrets`,
    signal ? { signal } : {},
  );
}

export function setPluginSecret(storeId: string, pluginId: string, name: string, value: string) {
  return pluginRequest<void>(storeId, `/${pluginId}/secrets/${encodeURIComponent(name)}`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ value }),
  });
}

export function deletePluginSecret(storeId: string, pluginId: string, name: string) {
  return pluginRequest<void>(storeId, `/${pluginId}/secrets/${encodeURIComponent(name)}`, {
    method: "DELETE",
  });
}
