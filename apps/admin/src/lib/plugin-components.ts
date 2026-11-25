import type {
  JsonValue,
  PageBlock,
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
/** Prefix of a page block type owned by a plugin. */
const PLUGIN_BLOCK_PREFIX = "plugin:";

/**
 * Block type stored in page JSON for a plugin component.
 *
 * Namespacing keeps two plugins free to declare the same component id and
 * makes a saved block self-describing when the plugin is gone.
 */
export function pluginBlockType(
  contribution: Pick<PluginComponentContribution, "plugin_id" | "id">,
): string {
  return `${PLUGIN_BLOCK_PREFIX}${contribution.plugin_id}:${contribution.id}`;
}

/** True when a page block type belongs to a plugin. */
export function isPluginBlockType(type: string): boolean {
  return type.startsWith(PLUGIN_BLOCK_PREFIX);
}

/** Parses a plugin block type back into its provider and component ids. */
export function parsePluginBlockType(
  type: string,
): { pluginId: string; componentId: string } | null {
  if (!isPluginBlockType(type)) return null;
  const rest = type.slice(PLUGIN_BLOCK_PREFIX.length);
  const separator = rest.indexOf(":");
  if (separator <= 0 || separator === rest.length - 1) return null;
  return { pluginId: rest.slice(0, separator), componentId: rest.slice(separator + 1) };
}

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
 * Each plugin component becomes a namespaced block type
 * (`plugin:{plugin}:{component}`), so declaration ids never collide across
 * plugins. A contribution whose namespaced type is already present is skipped;
 * contributions merge in input order and the base manifest is never mutated.
 * Components whose category is unknown are grouped under "Plugin components".
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
    if (typeof contribution.id !== "string" || contribution.id.length === 0) continue;
    if (typeof contribution.plugin_id !== "string" || contribution.plugin_id.length === 0) {
      continue;
    }
    const type = pluginBlockType(contribution);
    if (claimed.has(type)) continue;
    claimed.add(type);

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
    components[type] = component;

    const existing = categories[category];
    categories[category] = {
      title:
        existing !== undefined && typeof existing.title === "string"
          ? existing.title
          : PLUGIN_CATEGORY_TITLE,
      components: [...(Array.isArray(existing?.components) ? existing.components : []), type],
    };
    merged = true;
  }

  if (!merged) return base;
  return { ...base, categories, components };
}

/** A saved page block whose plugin component is no longer contributed. */
export interface MissingPluginBlock {
  /** Saved block type, for example `plugin:chat-widget:livechat-button`. */
  type: string;
  /** Providing plugin id parsed from the type. */
  pluginId: string;
  /** Component id parsed from the type. */
  componentId: string;
}

function collectMissingBlocks(
  blocks: readonly PageBlock[],
  available: ReadonlySet<string>,
  missing: Map<string, MissingPluginBlock>,
): void {
  for (const block of blocks) {
    if (isPluginBlockType(block.type) && !available.has(block.type)) {
      const parsed = parsePluginBlockType(block.type);
      if (parsed && !missing.has(block.type)) {
        missing.set(block.type, { type: block.type, ...parsed });
      }
    }
    for (const children of Object.values(block.slots ?? {})) {
      collectMissingBlocks(children, available, missing);
    }
  }
}

/**
 * Finds plugin blocks saved in a page that the current catalog cannot render.
 *
 * A plugin component disappears when the plugin is disabled, removed or
 * upgraded without the contribution. The page data stays untouched; the
 * editor uses this list to explain the placeholder instead of failing to load
 * the page.
 */
export function missingPluginBlocks(
  content: readonly PageBlock[],
  zones: readonly PageBlock[][],
  available: readonly string[],
): MissingPluginBlock[] {
  const renderable = new Set(available);
  const missing = new Map<string, MissingPluginBlock>();
  for (const block of content) collectMissingBlocks([block], renderable, missing);
  for (const blocks of zones) collectMissingBlocks(blocks, renderable, missing);
  return [...missing.values()];
}
