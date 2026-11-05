import type {
  JsonValue,
  PageBlock,
  PageBody,
  PageThemeRef,
  ThemeEditorComponent,
  ThemeEditorField,
  ThemeEditorManifest,
  ThemeEditorSlot,
} from "@fastcart/shared-types";

export const CURRENT_PAGE_SCHEMA_VERSION = 1;

export interface PaletteEntry {
  type: string;
  title: string;
  component: ThemeEditorComponent;
}

export interface PaletteGroup {
  key: string;
  title: string;
  entries: PaletteEntry[];
}

export interface ParsedJson {
  ok: boolean;
  value?: JsonValue;
  error?: string;
}

export function isJsonObject(value: unknown): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function asJsonObject(value: unknown): Record<string, JsonValue> {
  return isJsonObject(value) ? value : {};
}

export function cloneJson<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function prettyJson(value: unknown): string {
  try {
    return JSON.stringify(value ?? null, null, 2);
  } catch {
    return "null";
  }
}

export function parseJsonText(text: string): ParsedJson {
  try {
    return { ok: true, value: JSON.parse(text) as JsonValue };
  } catch (cause) {
    return { ok: false, error: cause instanceof Error ? cause.message : "Invalid JSON" };
  }
}

/** Normalizes a stored body into the structured editor model. */
export function normalizePageBody(value: JsonValue | null | undefined): PageBody {
  const record = asJsonObject(value);
  const root = asJsonObject(record.root);
  const theme = asJsonObject(record.theme);
  const zones: Record<string, PageBlock[]> = {};
  for (const [key, entries] of Object.entries(asJsonObject(record.zones))) {
    if (Array.isArray(entries)) {
      zones[key] = entries
        .map((entry) => normalizeBlock(entry))
        .filter((entry): entry is PageBlock => entry !== null);
    }
  }
  return {
    schema_version:
      typeof record.schema_version === "number"
        ? record.schema_version
        : CURRENT_PAGE_SCHEMA_VERSION,
    theme:
      typeof theme.id === "string" && typeof theme.version === "string"
        ? {
            id: theme.id,
            version: theme.version,
            ...(typeof theme.schema_version === "number"
              ? { schema_version: theme.schema_version }
              : {}),
          }
        : null,
    content: Array.isArray(record.content)
      ? record.content
          .map((entry) => normalizeBlock(entry))
          .filter((entry): entry is PageBlock => entry !== null)
      : [],
    root: { props: isJsonObject(root.props) ? (root.props as JsonValue) : {} },
    zones,
  };
}

function normalizeBlock(value: unknown): PageBlock | null {
  const record = asJsonObject(value);
  if (typeof record.type !== "string" || record.type.length === 0) return null;
  const slots: Record<string, PageBlock[]> = {};
  for (const [name, entries] of Object.entries(asJsonObject(record.slots))) {
    if (!Array.isArray(entries)) continue;
    slots[name] = entries
      .map((entry) => normalizeBlock(entry))
      .filter((entry): entry is PageBlock => entry !== null);
  }
  return {
    type: record.type,
    props: isJsonObject(record.props) ? (record.props as JsonValue) : {},
    slots,
  };
}

export function pageThemeRef(
  theme: { manifest: { id: string; version: string }; schema_version?: number } | null,
): PageThemeRef | null {
  if (!theme) return null;
  return {
    id: theme.manifest.id,
    version: theme.manifest.version,
    ...(typeof theme.schema_version === "number" ? { schema_version: theme.schema_version } : {}),
  };
}

export function blockId(block: PageBlock): string | null {
  const props = asJsonObject(block.props);
  return typeof props.id === "string" && props.id.length > 0 ? props.id : null;
}

export function blockProps(block: PageBlock): Record<string, JsonValue> {
  return asJsonObject(block.props);
}

export function collectBlockIds(blocks: PageBlock[], into = new Set<string>()): Set<string> {
  for (const block of blocks) {
    const id = blockId(block);
    if (id) into.add(id);
    for (const children of Object.values(block.slots ?? {})) {
      collectBlockIds(children, into);
    }
  }
  return into;
}

function uniqueBlockId(type: string, existing: Set<string>): string {
  const base =
    type
      .replace(/[^a-zA-Z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .toLowerCase() || "block";
  let candidate = base;
  let suffix = 1;
  while (existing.has(candidate)) {
    candidate = `${base}-${suffix}`;
    suffix += 1;
  }
  return candidate;
}

export function buildBlock(
  type: string,
  manifest: ThemeEditorManifest | null,
  existing: Set<string>,
): PageBlock {
  const component = manifest?.components?.[type];
  const defaults = isJsonObject(component?.defaultProps)
    ? cloneJson(component?.defaultProps as Record<string, JsonValue>)
    : {};
  const id = uniqueBlockId(type, existing);
  const props: Record<string, JsonValue> = { ...defaults, id };
  const slots: Record<string, PageBlock[]> = {};
  for (const slot of component?.slots ?? []) {
    slots[slot.name] = [];
  }
  return { type, props, slots };
}

export function findBlock(blocks: PageBlock[], id: string): PageBlock | null {
  for (const block of blocks) {
    if (blockId(block) === id) return block;
    for (const children of Object.values(block.slots ?? {})) {
      const found = findBlock(children, id);
      if (found) return found;
    }
  }
  return null;
}

export function findBlockPath(blocks: PageBlock[], id: string): PageBlock[] {
  for (const block of blocks) {
    if (blockId(block) === id) return [block];
    for (const children of Object.values(block.slots ?? {})) {
      const path = findBlockPath(children, id);
      if (path.length > 0) return [block, ...path];
    }
  }
  return [];
}

function moveWithin(blocks: PageBlock[], index: number, direction: -1 | 1): PageBlock[] {
  const target = index + direction;
  if (target < 0 || target >= blocks.length) return blocks;
  const copy = [...blocks];
  const entry = copy[index];
  const swap = copy[target];
  if (!entry || !swap) return blocks;
  copy[index] = swap;
  copy[target] = entry;
  return copy;
}

export function moveBlock(blocks: PageBlock[], id: string, direction: -1 | 1): PageBlock[] {
  const index = blocks.findIndex((block) => blockId(block) === id);
  if (index >= 0) return moveWithin(blocks, index, direction);

  let changed = false;
  const mapped = blocks.map((block) => {
    const slots = block.slots;
    if (!slots) return block;
    let slotChanged = false;
    const nextSlots: Record<string, PageBlock[]> = {};
    for (const [name, children] of Object.entries(slots)) {
      const nextChildren = moveBlock(children, id, direction);
      if (nextChildren !== children) {
        slotChanged = true;
        changed = true;
      }
      nextSlots[name] = nextChildren;
    }
    return slotChanged ? { ...block, slots: nextSlots } : block;
  });
  return changed ? mapped : blocks;
}

export function removeBlock(blocks: PageBlock[], id: string): PageBlock[] {
  const kept = blocks.filter((block) => blockId(block) !== id);
  if (kept.length !== blocks.length) return kept;

  let changed = false;
  const mapped = blocks.map((block) => {
    const slots = block.slots;
    if (!slots) return block;
    let slotChanged = false;
    const nextSlots: Record<string, PageBlock[]> = {};
    for (const [name, children] of Object.entries(slots)) {
      const nextChildren = removeBlock(children, id);
      if (nextChildren !== children) {
        slotChanged = true;
        changed = true;
      }
      nextSlots[name] = nextChildren;
    }
    return slotChanged ? { ...block, slots: nextSlots } : block;
  });
  return changed ? mapped : blocks;
}

export function appendBlock(blocks: PageBlock[], block: PageBlock): PageBlock[] {
  return [...blocks, block];
}

export function appendBlockToSlot(
  blocks: PageBlock[],
  parentId: string,
  slotName: string,
  block: PageBlock,
): PageBlock[] {
  let changed = false;
  const mapped = blocks.map((entry) => {
    if (blockId(entry) === parentId) {
      changed = true;
      const slots = entry.slots ?? {};
      return {
        ...entry,
        slots: {
          ...slots,
          [slotName]: [...(slots[slotName] ?? []), block],
        },
      };
    }
    const slots = entry.slots;
    if (!slots) return entry;
    let slotChanged = false;
    const nextSlots: Record<string, PageBlock[]> = {};
    for (const [name, children] of Object.entries(slots)) {
      const nextChildren = appendBlockToSlot(children, parentId, slotName, block);
      if (nextChildren !== children) {
        slotChanged = true;
        changed = true;
      }
      nextSlots[name] = nextChildren;
    }
    return slotChanged ? { ...entry, slots: nextSlots } : entry;
  });
  return changed ? mapped : blocks;
}

export function updateBlockProps(
  blocks: PageBlock[],
  id: string,
  mutate: (props: Record<string, JsonValue>) => Record<string, JsonValue>,
): PageBlock[] {
  let changed = false;
  const mapped = blocks.map((block) => {
    if (blockId(block) === id) {
      changed = true;
      return { ...block, props: mutate({ ...blockProps(block) }) };
    }
    const slots = block.slots;
    if (!slots) return block;
    let slotChanged = false;
    const nextSlots: Record<string, PageBlock[]> = {};
    for (const [name, children] of Object.entries(slots)) {
      const nextChildren = updateBlockProps(children, id, mutate);
      if (nextChildren !== children) {
        slotChanged = true;
        changed = true;
      }
      nextSlots[name] = nextChildren;
    }
    return slotChanged ? { ...block, slots: nextSlots } : block;
  });
  return changed ? mapped : blocks;
}

export function componentTitle(manifest: ThemeEditorManifest | null, type: string): string {
  return manifest?.components?.[type]?.title ?? type;
}

export function componentFields(
  manifest: ThemeEditorManifest | null,
  type: string,
): Record<string, ThemeEditorField> {
  return manifest?.components?.[type]?.fields ?? {};
}

export function componentSlots(
  manifest: ThemeEditorManifest | null,
  type: string,
): ThemeEditorSlot[] {
  return manifest?.components?.[type]?.slots ?? [];
}

export function paletteGroups(manifest: ThemeEditorManifest | null): PaletteGroup[] {
  const components = manifest?.components ?? {};
  const assigned = new Set<string>();
  const groups: PaletteGroup[] = [];

  for (const [key, category] of Object.entries(manifest?.categories ?? {})) {
    const entries: PaletteEntry[] = [];
    for (const type of category.components ?? []) {
      const component = components[type];
      if (!component) continue;
      assigned.add(type);
      entries.push({ type, title: component.title, component });
    }
    if (entries.length > 0) groups.push({ key, title: category.title, entries });
  }

  const remaining = Object.entries(components)
    .filter(([type]) => !assigned.has(type))
    .map(([type, component]) => ({ type, title: component.title, component }));
  if (remaining.length > 0) {
    groups.push({ key: "__other", title: "Other components", entries: remaining });
  }
  return groups;
}

export function slotEntries(
  manifest: ThemeEditorManifest | null,
  slot: ThemeEditorSlot,
): PaletteEntry[] {
  const components = manifest?.components ?? {};
  const allowed = slot.allowed_components;
  return Object.entries(components)
    .filter(([type]) => allowed === undefined || allowed === null || allowed.includes(type))
    .map(([type, component]) => ({ type, title: component.title, component }));
}

export function fieldDefault(field: ThemeEditorField): JsonValue | undefined {
  if (field.default !== undefined) return cloneJson(field.default);
  switch (field.type) {
    case "number":
      return typeof field.min === "number" ? field.min : 0;
    case "boolean":
      return false;
    case "array":
      return [];
    case "object":
      return {};
    case "select":
      return field.options?.[0]?.value ?? "";
    case "data_source":
    case "asset": {
      const source = field.source ?? (field.type === "asset" ? "asset" : undefined);
      return source ? { source } : undefined;
    }
    default:
      return "";
  }
}

/** Describes an item for a data-source picker. */
export function bindingId(value: JsonValue | undefined): string | null {
  if (typeof value === "string" && value.length > 0) return value;
  if (!isJsonObject(value)) return null;
  return typeof value.id === "string" ? value.id : null;
}

export function bindingSource(value: JsonValue | undefined): string | null {
  if (!isJsonObject(value)) return null;
  return typeof value.source === "string" ? value.source : null;
}
