import type {
  JsonValue,
  ThemeEditorCategory,
  ThemeEditorComponent,
  ThemeEditorField,
  ThemeEditorManifest,
  ThemeEditorSlot,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";
import { cloneJson } from "@/lib/theme-editor";

/** JSON object shape used for plugin-provided editor metadata. */
export type JsonObject = { [key: string]: JsonValue };

/** A capability a storefront component depends on at runtime. */
export interface PluginComponentRequirement {
  id: string;
  version?: number | null;
  optional?: boolean;
}

/** One storefront component contributed by an enabled plugin. */
export interface PluginComponentContribution {
  plugin_id: string;
  plugin_name: string;
  plugin_version: string;
  digest: string;
  id: string;
  title: string;
  category: string | null;
  editor_manifest: JsonObject | null;
  entry: string | null;
  requires: PluginComponentRequirement[];
}

/** Response from the plugin component catalog endpoint. */
export interface PluginComponentsResponse {
  components: PluginComponentContribution[];
}

/** Category key and title used when a component declares no known category. */
const PLUGIN_CATEGORY_KEY = "plugins";
const PLUGIN_CATEGORY_TITLE = "Plugin components";

function isJsonObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Loads the storefront components enabled for a store.
 *
 * Catalog failures do not make page editing impossible, so callers usually
 * fall back to an empty list and keep the built-in components available.
 */
export async function listPluginComponents(
  storeId: string,
  signal?: AbortSignal,
): Promise<PluginComponentContribution[]> {
  const headers = new Headers();
  headers.set("x-store-id", storeId);
  const csrf = readCookie("fc_csrf");
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/admin/components`, {
    credentials: "include",
    headers,
    ...(signal ? { signal } : {}),
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(apiErrorMessage(body, `Plugin component request failed (${response.status})`));
  }
  const body = (await response.json()) as PluginComponentsResponse | null;
  return Array.isArray(body?.components) ? body.components : [];
}

/**
 * Keeps the field entries that are objects with a string type. Options are
 * editor hints, so a non-array value is dropped instead of trusted.
 */
function sanitizeFields(value: JsonValue | undefined): Record<string, ThemeEditorField> | null {
  if (!isJsonObject(value)) return null;
  const fields: Record<string, ThemeEditorField> = {};
  for (const [name, entry] of Object.entries(value)) {
    if (!isJsonObject(entry)) continue;
    if (typeof entry.type !== "string") continue;
    const field = { ...entry } as unknown as ThemeEditorField;
    if (!Array.isArray(entry.options)) delete field.options;
    fields[name] = field;
  }
  return fields;
}

/**
 * Keeps named slots and only accepts an allowed component list when every
 * entry is a component type string.
 */
function sanitizeSlots(value: JsonValue | undefined): ThemeEditorSlot[] | null {
  if (!Array.isArray(value)) return null;
  const slots: ThemeEditorSlot[] = [];
  for (const entry of value) {
    if (!isJsonObject(entry)) continue;
    if (typeof entry.name !== "string") continue;
    const slot: ThemeEditorSlot = { name: entry.name };
    if (typeof entry.title === "string") slot.title = entry.title;
    const allowed = entry.allowed_components;
    if (Array.isArray(allowed) && allowed.every((item) => typeof item === "string")) {
      slot.allowed_components = allowed as string[];
    }
    slots.push(slot);
  }
  return slots;
}

/**
 * Resolves the palette category for a contribution.
 *
 * The manifest category wins over the contribution category, and either one
 * must match a category already declared by the built-in manifest. Everything
 * else lands in the shared plugin category.
 */
function resolveCategory(
  baseCategories: Record<string, ThemeEditorCategory>,
  manifest: JsonObject,
  contribution: PluginComponentContribution,
): string {
  const declared =
    typeof manifest.category === "string" && manifest.category.length > 0
      ? manifest.category
      : contribution.category;
  if (typeof declared === "string" && Object.hasOwn(baseCategories, declared)) {
    return declared;
  }
  return PLUGIN_CATEGORY_KEY;
}

/**
 * Appends plugin components to the page editor manifest.
 *
 * Built-in components win every id collision, plugin contributions are merged
 * in input order, and the base manifest is never mutated. Components whose
 * category is unknown are grouped under "Plugin components".
 */
export function mergePluginComponents(
  base: ThemeEditorManifest,
  contributions: readonly PluginComponentContribution[],
): ThemeEditorManifest {
  if (contributions.length === 0) return base;

  const baseComponents = base.components ?? {};
  const baseCategories = base.categories ?? {};
  const claimed = new Set(Object.keys(baseComponents));
  const components: Record<string, ThemeEditorComponent> = { ...baseComponents };
  const categories: Record<string, ThemeEditorCategory> = { ...baseCategories };
  let merged = false;

  for (const contribution of contributions) {
    const id = contribution.id;
    if (typeof id !== "string" || id.length === 0) continue;
    if (claimed.has(id)) continue;
    claimed.add(id);

    const manifest: JsonObject = isJsonObject(contribution.editor_manifest)
      ? contribution.editor_manifest
      : {};
    const component: ThemeEditorComponent = {
      title: typeof manifest.title === "string" ? manifest.title : contribution.title,
    };
    if (typeof manifest.description === "string") component.description = manifest.description;
    if (typeof manifest.icon === "string") component.icon = manifest.icon;
    const fields = sanitizeFields(manifest.fields);
    if (fields !== null) component.fields = fields;
    const slots = sanitizeSlots(manifest.slots);
    if (slots !== null) component.slots = slots;
    if (manifest.defaultProps !== undefined) {
      component.defaultProps = cloneJson(manifest.defaultProps);
    }

    const category = resolveCategory(baseCategories, manifest, contribution);
    component.category = category;
    components[id] = component;

    const existing = categories[category];
    categories[category] = {
      title:
        existing !== undefined && typeof existing.title === "string"
          ? existing.title
          : PLUGIN_CATEGORY_TITLE,
      components: [...(Array.isArray(existing?.components) ? existing.components : []), id],
    };
    merged = true;
  }

  if (!merged) return base;
  return { ...base, categories, components };
}
