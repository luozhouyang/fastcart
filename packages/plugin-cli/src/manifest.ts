/** Minimal manifest validation shared by pack, doctor and dev. */

import { existsSync } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { join, normalize, sep } from "node:path";

export const ENABLED_TARGETS = [
  "admin.nav.section",
  "admin.page.overview.after-header",
  "admin.page.orders.details.side",
  "admin.page.products.details.side",
  "admin.page.customers.details.side",
  "admin.page.settings.tabs",
] as const;

export interface AdminExtension {
  target: string;
  title?: string;
  label?: string;
  icon?: string;
  route?: string;
  entry?: string;
  height?: number;
  priority?: number;
}

export interface StorefrontExtension {
  kind: "embed";
  placement: "head:start" | "head:end" | "body:start" | "body:end";
  entry: string;
  css?: string;
  pages?: string[];
  exclude_paths?: string[];
  height_hint?: number;
}

export interface CapabilityRequirement {
  id: string;
  version?: number;
  optional?: boolean;
}

export interface CapabilityContribution {
  kind: "storefront_capability";
  id: string;
  version: number;
  title: string;
  config_schema?: Record<string, unknown>;
}

export interface ComponentContribution {
  kind: "storefront_component";
  id: string;
  title: string;
  category?: string;
  editor_manifest?: Record<string, unknown>;
  entry?: string;
  requires?: CapabilityRequirement[];
}

export type Contribution = CapabilityContribution | ComponentContribution;

export interface PluginManifest {
  schema_version: string;
  id: string;
  name: string;
  version: string;
  api_version: number;
  runtime: { engine: string; compatibility_date: string; compatibility_flags?: string[] };
  entry: { type: string; path: string; export?: string };
  capabilities?: Record<string, string[]>;
  extensions?: { admin?: AdminExtension[]; storefront?: StorefrontExtension[] };
  provides?: Contribution[];
  settings?: { schema: Record<string, unknown>; ui?: string };
  rpc?: string[];
  rpc_permissions?: Record<string, string>;
  jobs?: Array<{ name: string; schedule?: string }>;
}

const IDENTIFIER = /^[a-z0-9](?:[a-z0-9._-]{0,118}[a-z0-9])?$/;
const MAX_CONTRIBUTIONS = 20;

export class PluginValidationError extends Error {
  override name = "PluginValidationError";
}

export async function readManifest(root: string): Promise<PluginManifest> {
  const path = join(root, "plugin.json");
  let raw: string;
  try {
    raw = await readFile(path, "utf8");
  } catch {
    throw new PluginValidationError(`missing plugin.json in ${root}`);
  }
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch (error) {
    throw new PluginValidationError(`plugin.json is not valid JSON: ${String(error)}`);
  }
  if (typeof value !== "object" || value === null) {
    throw new PluginValidationError("plugin.json must be an object");
  }
  const manifest = value as PluginManifest;
  validateManifest(manifest);
  return manifest;
}

export function validateManifest(manifest: PluginManifest): void {
  const fail = (message: string): never => {
    throw new PluginValidationError(message);
  };
  if (manifest.schema_version !== "1") fail('schema_version must be "1"');
  if (typeof manifest.id !== "string" || !IDENTIFIER.test(manifest.id)) {
    fail("id must be lowercase alphanumeric with . _ - separators");
  }
  if (!manifest.name) fail("name is required");
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(manifest.version)) {
    fail("version must be semver (x.y.z)");
  }
  if (manifest.api_version !== 2) fail("api_version must be 2");
  if (manifest.runtime?.engine !== "workerd") fail("runtime.engine must be workerd");
  if (!/^\d{4}-\d{2}-\d{2}$/.test(manifest.runtime?.compatibility_date ?? "")) {
    fail("runtime.compatibility_date must be YYYY-MM-DD");
  }
  const entryPath = manifest.entry?.path;
  if (!entryPath || !safePath(entryPath) || !/\.(?:js|mjs)$/.test(entryPath)) {
    fail("entry.path must be a relative .js/.mjs path");
  }
  if (manifest.entry?.type !== "worker") fail("entry.type must be worker");
  for (const extension of manifest.extensions?.admin ?? []) {
    if (!(ENABLED_TARGETS as readonly string[]).includes(extension.target)) {
      fail(`unknown admin target: ${extension.target}`);
    }
    if (extension.entry && !safePath(extension.entry))
      fail(`unsafe admin entry: ${extension.entry}`);
    if (extension.route && !extension.route.startsWith("/")) {
      fail(`admin route must be site-relative: ${extension.route}`);
    }
  }
  for (const extension of manifest.extensions?.storefront ?? []) {
    if (extension.kind !== "embed") fail(`unsupported storefront kind: ${String(extension.kind)}`);
    if (!["head:start", "head:end", "body:start", "body:end"].includes(extension.placement)) {
      fail(`unsupported placement: ${String(extension.placement)}`);
    }
    if (!safePath(extension.entry)) fail(`unsafe storefront entry: ${extension.entry}`);
    if (extension.css && !safePath(extension.css)) fail(`unsafe storefront css: ${extension.css}`);
  }
  if (manifest.settings?.ui && !safePath(manifest.settings.ui)) {
    fail(`unsafe settings UI path: ${manifest.settings.ui}`);
  }
  for (const scope of manifest.capabilities?.secrets ?? []) {
    if (scope !== "secrets.read") fail(`unsupported secret scope: ${scope}`);
  }
  for (const method of manifest.rpc ?? []) {
    if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(method)) fail(`invalid RPC method: ${method}`);
  }
  for (const [method, permission] of Object.entries(manifest.rpc_permissions ?? {})) {
    if (!(manifest.rpc ?? []).includes(method)) {
      fail(`RPC permission names an undeclared method: ${method}`);
    }
    if (!/^[a-z0-9_]+(\.[a-z0-9_]+)+$/.test(permission)) {
      fail(`invalid permission for RPC method ${method}: ${permission}`);
    }
  }
  validateContributions(manifest, fail);
}

function validateContributions(manifest: PluginManifest, fail: (message: string) => never): void {
  const provides: unknown = manifest.provides ?? [];
  if (!Array.isArray(provides)) fail("provides must be an array");
  if (provides.length > MAX_CONTRIBUTIONS) {
    fail(`at most ${MAX_CONTRIBUTIONS} contributions are allowed`);
  }
  const componentScope = (manifest.capabilities?.storefront ?? []).includes("storefront.component");
  const seen = new Set<string>();
  for (const raw of provides as unknown[]) {
    if (!isJsonObject(raw)) fail("contribution must be an object");
    const contribution = raw as unknown as Contribution;
    if (
      contribution.kind !== "storefront_capability" &&
      contribution.kind !== "storefront_component"
    ) {
      fail(`unsupported contribution kind: ${String((contribution as { kind?: unknown }).kind)}`);
    }
    if (typeof contribution.id !== "string" || !IDENTIFIER.test(contribution.id)) {
      fail("contribution id must be lowercase alphanumeric with . _ - separators");
    }
    if (contribution.kind === "storefront_capability") {
      if (!Number.isInteger(contribution.version) || contribution.version < 1) {
        fail(`capability ${contribution.id} version must be a positive integer`);
      }
      if (contribution.config_schema !== undefined && !isJsonObject(contribution.config_schema)) {
        fail(`capability ${contribution.id} config_schema must be a JSON object`);
      }
      const key = `${contribution.kind}:${contribution.id}:${contribution.version}`;
      if (seen.has(key)) fail(`duplicate contribution: ${key}`);
      seen.add(key);
      continue;
    }
    if (contribution.editor_manifest !== undefined && !isJsonObject(contribution.editor_manifest)) {
      fail(`component ${contribution.id} editor_manifest must be a JSON object`);
    }
    if (
      contribution.entry !== undefined &&
      (!safePath(contribution.entry) || !/\.(?:js|mjs)$/.test(contribution.entry))
    ) {
      fail(`component ${contribution.id} entry must be a relative .js/.mjs path`);
    }
    const requirementIds = new Set<string>();
    for (const requirement of contribution.requires ?? []) {
      if (
        !isJsonObject(requirement) ||
        typeof requirement.id !== "string" ||
        !IDENTIFIER.test(requirement.id)
      ) {
        fail(`component ${contribution.id} has an invalid requirement id`);
      }
      if (
        requirement.version !== undefined &&
        (!Number.isInteger(requirement.version) || requirement.version < 1)
      ) {
        fail(`requirement ${requirement.id} version must be a positive integer`);
      }
      if (requirementIds.has(requirement.id)) {
        fail(`duplicate requirement: ${requirement.id}`);
      }
      requirementIds.add(requirement.id);
    }
    if (!componentScope) {
      fail(`component ${contribution.id} requires the storefront.component scope`);
    }
    const key = `${contribution.kind}:${contribution.id}`;
    if (seen.has(key)) fail(`duplicate contribution: ${key}`);
    seen.add(key);
  }
}

function isJsonObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function safePath(value: string): boolean {
  if (value.length === 0 || value.startsWith("/") || value.includes("\\")) return false;
  const normalized = normalize(value);
  return (
    normalized !== ".." && !normalized.startsWith(`..${sep}`) && !normalized.includes(`..${sep}`)
  );
}

/** Files the manifest promises to ship. */
export function referencedFiles(manifest: PluginManifest): string[] {
  const files = [manifest.entry.path];
  for (const extension of manifest.extensions?.admin ?? []) {
    if (extension.entry) files.push(extension.entry);
  }
  for (const extension of manifest.extensions?.storefront ?? []) {
    files.push(extension.entry);
    if (extension.css) files.push(extension.css);
  }
  for (const contribution of manifest.provides ?? []) {
    if (contribution.kind === "storefront_component" && contribution.entry) {
      files.push(contribution.entry);
    }
  }
  if (manifest.settings?.ui) files.push(manifest.settings.ui);
  return [...new Set(files)];
}

export async function missingReferences(root: string, manifest: PluginManifest): Promise<string[]> {
  const missing: string[] = [];
  for (const file of referencedFiles(manifest)) {
    try {
      const info = await stat(join(root, file));
      if (!info.isFile()) missing.push(file);
    } catch {
      missing.push(file);
    }
  }
  return missing;
}

/** Resolves the source entry, accepting a neighbouring `.ts` file. */
export function sourceEntry(root: string, entryPath: string): string {
  if (existsSync(join(root, entryPath))) return join(root, entryPath);
  const ts = join(root, entryPath.replace(/\.(?:js|mjs)$/, ".ts"));
  if (existsSync(ts)) return ts;
  const tsx = join(root, entryPath.replace(/\.(?:js|mjs)$/, ".tsx"));
  if (existsSync(tsx)) return tsx;
  throw new PluginValidationError(`entry source not found: ${entryPath} (or .ts/.tsx)`);
}
