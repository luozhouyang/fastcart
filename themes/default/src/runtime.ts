import type { PageBlock, PageBody, RenderMode, RenderThemeContext } from "@fastcart/theme-sdk";

/** Deterministic browser entry emitted by the client Vite build. */
export const CLIENT_ENTRY = "client.js";

/** Returns true for a plain JSON object. */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Coerces an incoming page body into the structure the renderer expects. */
export function normalizePageBody(value: unknown): PageBody {
  if (!isRecord(value)) {
    return { schema_version: 1, content: [], zones: {} };
  }
  const content = Array.isArray(value.content) ? value.content.filter(isPageBlock) : [];
  const zones: Record<string, PageBlock[]> = {};
  if (isRecord(value.zones)) {
    for (const [name, blocks] of Object.entries(value.zones)) {
      if (Array.isArray(blocks) && name.length > 0) {
        zones[name] = blocks.filter(isPageBlock);
      }
    }
  }
  return {
    schema_version: typeof value.schema_version === "number" ? value.schema_version : 1,
    ...(isRecord(value.theme) &&
    typeof value.theme.id === "string" &&
    typeof value.theme.version === "string"
      ? {
          theme: {
            id: value.theme.id,
            version: value.theme.version,
            ...(typeof value.theme.schema_version === "number"
              ? { schema_version: value.theme.schema_version }
              : {}),
          },
        }
      : {}),
    ...(isRecord(value.root) ? { root: value.root } : {}),
    content,
    zones,
  };
}

function isPageBlock(value: unknown): value is PageBlock {
  return isRecord(value) && typeof value.type === "string" && value.type.length > 0;
}

/** Extracts a primitive settings object from the runtime theme. */
export function normalizeSettings(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : {};
}

/** Resolves the API origin configured in theme settings, if any. */
export function apiBaseUrlFromSettings(settings: Record<string, unknown>): string | undefined {
  for (const key of ["apiBaseUrl", "api_base_url", "api_base"]) {
    const value = settings[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim().replace(/\/+$/, "");
    }
  }
  return undefined;
}

/** Normalizes the runtime mode sent by the host. */
export function normalizeMode(value: unknown): RenderMode {
  return value === "preview" || value === "editor" ? value : "production";
}

/**
 * Builds the browser module URL for the client bundle.
 *
 * The host-provided `assets_base_url` already points at the directory that
 * serves the client output (for example `/api/v1/store/theme-assets`), so the
 * entry is appended directly. No script is emitted when the host did not
 * provide a public asset base: a broken absolute URL would be worse than
 * losing hydration.
 */
export function clientScriptUrl(theme: RenderThemeContext | null | undefined): string | null {
  const base = theme?.assets_base_url?.trim();
  if (!base) {
    return null;
  }
  return `${base.replace(/\/+$/, "")}/${CLIENT_ENTRY}`;
}
