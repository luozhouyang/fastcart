import { pluginRequest, type InstalledPlugin } from "@/lib/plugins";

/** Settings document stored for one plugin; shape is plugin-defined. */
export type PluginSettings = Record<string, unknown>;

/** One persisted event delivery row (`plugin_deliveries`). */
export interface DeliveryView {
  id: string;
  plugin_id: string;
  event_id: string;
  event_type: string;
  status: string;
  attempts: number;
  available_at: number;
  last_error: string | null;
  created_at: number;
}

/** One persisted job row (`plugin_jobs`). */
export interface JobView {
  id: string;
  plugin_id: string;
  name: string;
  status: string;
  attempts: number;
  available_at: number;
  last_error: string | null;
  created_at: number;
  updated_at: number;
}

/** One persisted log line (`plugin_logs`). */
export interface LogView {
  id: number;
  plugin_id: string;
  level: string;
  message: string;
  fields_json: string | null;
  created_at: number;
}

/** Health of the shared plugin runtime (`/plugins/runtime`). */
export interface PluginRuntimeStatus {
  running: boolean;
  generation: string | null;
  port: number | null;
  restarts: number;
  last_error: string | null;
  uptime_seconds: number;
}

/** UI extension declared by a manifest. */
export interface ManifestAdminExtension {
  target: string;
  title?: string | null;
  label?: string | null;
  icon?: string | null;
  route?: string | null;
  entry?: string | null;
  height?: number | null;
  priority?: number | null;
}

/** Storefront embed declared by a manifest. */
export interface ManifestStorefrontExtension {
  kind: string;
  placement: string;
  entry: string;
  css?: string | null;
  pages?: string[];
  exclude_paths?: string[];
  height_hint?: number | null;
}

/** Capability groups declared by a manifest. */
export interface ManifestCapabilities {
  data?: string[];
  storage?: string[];
  secrets?: string[];
  events?: string[];
  network?: string[];
  network_auth?: string[];
  admin?: string[];
  storefront?: string[];
}

/** One declared job (name plus optional cron schedule). */
export interface ManifestJob {
  name: string;
  schedule?: string | null;
}

/** Settings schema declaration. */
export interface ManifestSettings {
  schema: unknown;
  ui?: string | null;
}

/**
 * Validated manifest returned by `GET /plugins/{id}`.
 *
 * Only the fields the admin UI renders are typed; unknown declarations stay
 * available through the index signature so the Overview tab can show the
 * manifest verbatim.
 */
export interface ManifestView {
  schema_version: string;
  id: string;
  name: string;
  version: string;
  description?: string | null;
  author?: { name: string; url?: string | null } | null;
  api_version: number;
  runtime: {
    engine: string;
    compatibility_date: string;
    compatibility_flags?: string[];
  };
  entry: { type: string; path: string; export?: string | null };
  capabilities?: ManifestCapabilities;
  extensions?: {
    admin?: ManifestAdminExtension[];
    storefront?: ManifestStorefrontExtension[];
  };
  settings?: ManifestSettings | null;
  rpc?: string[];
  rpc_permissions?: Record<string, string>;
  jobs?: ManifestJob[];
  build?: { build_id: string; created_at?: string | null } | null;
  [key: string]: unknown;
}

/** Installation record plus its manifest. */
export interface PluginDetail {
  plugin: InstalledPlugin;
  manifest: ManifestView;
}

/** Loads one installed plugin with its manifest. */
export function getPlugin(storeId: string, pluginId: string, signal?: AbortSignal) {
  return pluginRequest<PluginDetail>(
    storeId,
    `/${encodeURIComponent(pluginId)}`,
    signal ? { signal } : {},
  );
}

/** Rolls the plugin back to its previously active artifact. */
export function rollbackPlugin(storeId: string, pluginId: string) {
  return pluginRequest<InstalledPlugin>(storeId, `/${encodeURIComponent(pluginId)}/rollback`, {
    method: "POST",
  });
}

/**
 * Uploads a new archive for an installed plugin.
 *
 * New capabilities are accepted explicitly so a version that widens its
 * grants is never upgraded into silently.
 */
export function upgradePlugin(
  storeId: string,
  pluginId: string,
  archive: File,
  acceptNewScopes = false,
) {
  const body = new FormData();
  body.append("archive", archive);
  const query = acceptNewScopes ? "?accept_new_scopes=true" : "";
  return pluginRequest<InstalledPlugin>(
    storeId,
    `/${encodeURIComponent(pluginId)}/upgrade${query}`,
    { method: "POST", body },
  );
}

/** Lists the most recent job rows for one plugin. */
export function listJobs(storeId: string, pluginId: string, signal?: AbortSignal) {
  return pluginRequest<JobView[]>(
    storeId,
    `/${encodeURIComponent(pluginId)}/jobs`,
    signal ? { signal } : {},
  );
}

/** Enqueues a declared job for immediate execution. */
export function runJob(storeId: string, pluginId: string, name: string) {
  return pluginRequest<void>(
    storeId,
    `/${encodeURIComponent(pluginId)}/jobs/${encodeURIComponent(name)}/run`,
    { method: "POST" },
  );
}

/** Lists the most recent log lines for one plugin. */
export function listLogs(storeId: string, pluginId: string, signal?: AbortSignal) {
  return pluginRequest<LogView[]>(
    storeId,
    `/${encodeURIComponent(pluginId)}/logs`,
    signal ? { signal } : {},
  );
}

/** Deletes every stored log line for one plugin. */
export function clearLogs(storeId: string, pluginId: string) {
  return pluginRequest<void>(storeId, `/${encodeURIComponent(pluginId)}/logs`, {
    method: "DELETE",
  });
}

/** Lists the most recent event deliveries for one plugin. */
export function listDeliveries(storeId: string, pluginId: string, signal?: AbortSignal) {
  return pluginRequest<DeliveryView[]>(
    storeId,
    `/${encodeURIComponent(pluginId)}/deliveries`,
    signal ? { signal } : {},
  );
}

/** Schedules a persisted delivery for another attempt. */
export function replayDelivery(storeId: string, pluginId: string, deliveryId: string) {
  return pluginRequest<void>(
    storeId,
    `/${encodeURIComponent(pluginId)}/deliveries/${encodeURIComponent(deliveryId)}/replay`,
    { method: "POST" },
  );
}

/** Reads plugin runtime health. */
export function runtimeStatus(storeId: string, signal?: AbortSignal) {
  return pluginRequest<PluginRuntimeStatus>(storeId, "/runtime", signal ? { signal } : {});
}

/** Statuses the dispatch loop persists for deliveries and jobs. */
const ACTIVE_STATUSES = new Set(["pending", "running"]);
const FAILED_STATUSES = new Set(["failed", "dead"]);

/** Tone used by the admin pills for one persisted status string. */
export function statusTone(
  status: string,
): "brand" | "info" | "warning" | "destructive" | "secondary" {
  const normalized = status.toLowerCase();
  if (normalized === "done" || normalized === "completed" || normalized === "delivered") {
    return "brand";
  }
  if (normalized === "running") return "info";
  if (ACTIVE_STATUSES.has(normalized)) return "warning";
  if (FAILED_STATUSES.has(normalized)) return "destructive";
  return "secondary";
}

/** True while a delivery or job still has work left to do. */
export function isActiveStatus(status: string): boolean {
  return ACTIVE_STATUSES.has(status.toLowerCase());
}

/** Parses a stored log `fields_json` blob for display. */
export function parseLogFields(fieldsJson: string | null): Record<string, unknown> | null {
  if (fieldsJson === null || fieldsJson.length === 0) return null;
  try {
    const parsed: unknown = JSON.parse(fieldsJson);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return null;
    return parsed as Record<string, unknown>;
  } catch {
    return null;
  }
}
