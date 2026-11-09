import { createHash } from "node:crypto";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { dirname, join, posix, resolve, sep } from "node:path";

/** Manifest schema version emitted by this package. */
export const SCHEMA_VERSION = "1";
/** Name of the manifest inside an artifact. */
export const MANIFEST_FILE = "fastcart-ssr.json";

/** Runtime capabilities the FastCart gateway can provide today. */
export const SUPPORTED_CAPABILITIES = [
  "fetch",
  "streams",
  "web-crypto",
  "timers",
  "url",
  "nodejs-compat",
] as const;

export type ArtifactCapability = (typeof SUPPORTED_CAPABILITIES)[number];

export interface FrameworkMetadata {
  name: string;
  version: string;
}

export interface RuntimeMetadata {
  engine: "workerd";
  compatibility_date: string;
  compatibility_flags: string[];
  required_capabilities: ArtifactCapability[];
}

export interface EntryMetadata {
  type: "worker";
  path: string;
  export: string;
}

export interface AssetMetadata {
  directory: string;
  base_path: string;
}

export interface RenderingMetadata {
  mode: "static" | "ssr" | "hybrid";
  streaming: boolean;
}

export interface BuildMetadata {
  build_id: string;
  created_at: string;
}

/** Versioned contract between a framework build and FastCart. */
export interface ArtifactManifest {
  schema_version: string;
  artifact_id: string;
  framework: FrameworkMetadata;
  runtime: RuntimeMetadata;
  entry: EntryMetadata;
  assets: AssetMetadata;
  rendering: RenderingMetadata;
  build: BuildMetadata;
}

export interface CreateManifestInput {
  artifactId: string;
  framework: FrameworkMetadata;
  entry: EntryMetadata;
  assets: AssetMetadata;
  runtime?: Partial<RuntimeMetadata>;
  rendering?: Partial<RenderingMetadata>;
  build?: Partial<BuildMetadata>;
}

export interface ArtifactLimits {
  maxBytes: number;
  maxFiles: number;
}

export const DEFAULT_LIMITS: ArtifactLimits = {
  maxBytes: 100 * 1024 * 1024,
  maxFiles: 5_000,
};

export interface PackageArtifactInput {
  sourceDir: string;
  outputDir: string;
  manifest: ArtifactManifest;
  limits?: ArtifactLimits;
}

export interface PackagedArtifact {
  manifest: ArtifactManifest;
  digest: string;
  directory: string;
}

/** Validation or packaging failure. */
export class ArtifactBuildError extends Error {
  override name = "ArtifactBuildError";
}

/** Builds a manifest with FastCart defaults. */
export function createManifest(input: CreateManifestInput): ArtifactManifest {
  const manifest: ArtifactManifest = {
    schema_version: SCHEMA_VERSION,
    artifact_id: input.artifactId,
    framework: { ...input.framework },
    runtime: {
      engine: "workerd",
      compatibility_date: "2026-11-01",
      compatibility_flags: [],
      required_capabilities: [],
      ...input.runtime,
    },
    entry: { ...input.entry },
    assets: { ...input.assets },
    rendering: {
      mode: "hybrid",
      streaming: true,
      ...input.rendering,
    },
    build: {
      build_id: `build_${Date.now()}`,
      created_at: new Date().toISOString(),
      ...input.build,
    },
  };
  validateManifest(manifest);
  return manifest;
}

/** Validates an unknown JSON value and returns a typed manifest. */
export function validateManifest(value: unknown): ArtifactManifest {
  const root = record(value, "manifest");
  if (root.schema_version !== SCHEMA_VERSION) {
    fail(`unsupported schema_version: ${String(root.schema_version)}`);
  }
  const artifactId = stringField(root, "artifact_id");
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(artifactId)) {
    fail("artifact_id must be 1-128 safe characters");
  }
  const framework = frameworkMetadata(root.framework);
  const runtime = runtimeMetadata(root.runtime);
  const entry = entryMetadata(root.entry);
  const assets = assetMetadata(root.assets);
  const rendering = renderingMetadata(root.rendering);
  const build = buildMetadata(root.build);
  validateRelativePath(entry.path, "entry.path");
  if (!/\.(?:js|mjs)$/.test(entry.path)) {
    fail("entry.path must end in .js or .mjs");
  }
  validateRelativePath(assets.directory, "assets.directory");
  if (!assets.base_path.startsWith("/") || assets.base_path.startsWith("//")) {
    fail("assets.base_path must be an absolute URL path");
  }
  if (assets.base_path.includes("..")) {
    fail("assets.base_path must not contain ..");
  }
  return {
    schema_version: SCHEMA_VERSION,
    artifact_id: artifactId,
    framework,
    runtime,
    entry,
    assets,
    rendering,
    build,
  };
}

/** Reads and validates `fastcart-ssr.json` from an artifact directory. */
export async function readManifest(directory: string): Promise<ArtifactManifest> {
  const path = join(directory, MANIFEST_FILE);
  let raw: string;
  try {
    raw = await readFile(path, "utf8");
  } catch {
    throw new ArtifactBuildError(`artifact manifest not found at ${path}`);
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (error) {
    throw new ArtifactBuildError(`invalid artifact manifest JSON: ${(error as Error).message}`);
  }
  return validateManifest(parsed);
}

/** Writes a manifest with stable formatting. */
export async function writeManifest(directory: string, manifest: ArtifactManifest): Promise<void> {
  validateManifest(manifest);
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, MANIFEST_FILE), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

/** Computes the SHA-256 digest over sorted relative paths and file contents. */
export async function computeArtifactDigest(directory: string): Promise<string> {
  const files = await listFiles(directory);
  const hash = createHash("sha256");
  for (const relative of files) {
    const path = join(directory, ...relative.split("/"));
    hash.update(relative, "utf8");
    hash.update(Buffer.from([0]));
    hash.update(await readFile(path));
    hash.update(Buffer.from([0]));
  }
  return hash.digest("hex");
}

/** Validates a source tree, writes its manifest and stores it by digest. */
export async function packageArtifact(input: PackageArtifactInput): Promise<PackagedArtifact> {
  const manifest = validateManifest(input.manifest);
  const sourceDir = resolve(input.sourceDir);
  await assertDirectory(sourceDir, "sourceDir");
  await validateSourceTree(sourceDir, manifest, input.limits ?? DEFAULT_LIMITS);

  await mkdir(input.outputDir, { recursive: true });
  const staging = await mkdtemp(join(input.outputDir, ".staging-"));
  try {
    await copyTree(sourceDir, staging);
    await writeManifest(staging, manifest);
    const digest = await computeArtifactDigest(staging);
    const destination = join(input.outputDir, digest);
    if (await exists(destination)) {
      const existing = await computeArtifactDigest(destination);
      if (existing !== digest) {
        throw new ArtifactBuildError(`artifact at ${destination} has a different digest`);
      }
      await rm(staging, { recursive: true, force: true });
    } else {
      await rename(staging, destination);
    }
    return { manifest, digest, directory: destination };
  } catch (error) {
    await rm(staging, { recursive: true, force: true });
    throw error;
  }
}

async function validateSourceTree(
  sourceDir: string,
  manifest: ArtifactManifest,
  limits: ArtifactLimits,
): Promise<void> {
  const entry = safeJoin(sourceDir, manifest.entry.path);
  const entryStats = await lstat(entry).catch(() => null);
  if (!entryStats?.isFile()) {
    throw new ArtifactBuildError(`artifact entry not found: ${manifest.entry.path}`);
  }
  const assets = safeJoin(sourceDir, manifest.assets.directory);
  const assetsStats = await lstat(assets).catch(() => null);
  if (!assetsStats?.isDirectory()) {
    throw new ArtifactBuildError(
      `artifact asset directory not found: ${manifest.assets.directory}`,
    );
  }
  const files = await listFiles(sourceDir);
  if (files.length > limits.maxFiles) {
    throw new ArtifactBuildError(
      `artifact exceeds files limit: ${files.length} > ${limits.maxFiles}`,
    );
  }
  let total = 0;
  for (const relative of files) {
    total += (await stat(join(sourceDir, ...relative.split("/")))).size;
    if (total > limits.maxBytes) {
      throw new ArtifactBuildError(`artifact exceeds bytes limit: ${total} > ${limits.maxBytes}`);
    }
  }
}

async function listFiles(directory: string): Promise<string[]> {
  const output: string[] = [];
  async function walk(current: string): Promise<void> {
    const entries = await readdir(current, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      const path = join(current, entry.name);
      if (entry.isSymbolicLink()) {
        throw new ArtifactBuildError(`artifact contains a symlink: ${path}`);
      }
      if (entry.isDirectory()) {
        await walk(path);
      } else if (entry.isFile()) {
        output.push(
          path
            .slice(directory.length + 1)
            .split(sep)
            .join(posix.sep),
        );
      }
    }
  }
  await walk(directory);
  output.sort();
  return output;
}

async function copyTree(source: string, destination: string): Promise<void> {
  const entries = await readdir(source, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    const from = join(source, entry.name);
    const to = join(destination, entry.name);
    if (entry.isSymbolicLink()) {
      throw new ArtifactBuildError(`artifact contains a symlink: ${from}`);
    }
    if (entry.isDirectory()) {
      await mkdir(to, { recursive: true });
      await copyTree(from, to);
    } else if (entry.isFile()) {
      await mkdir(dirname(to), { recursive: true });
      await copyFile(from, to);
    }
  }
}

function safeJoin(root: string, relative: string): string {
  validateRelativePath(relative, "path");
  const target = resolve(root, ...relative.split("/"));
  if (target !== root && !target.startsWith(`${root}${sep}`)) {
    fail(`path escapes artifact root: ${relative}`);
  }
  return target;
}

function validateRelativePath(path: string, field: string): void {
  if (
    path.length === 0 ||
    path.startsWith("/") ||
    path.includes("\\") ||
    path.includes(":") ||
    path.split("/").some((part) => part.length === 0 || part === "..")
  ) {
    fail(`unsafe ${field}: ${path}`);
  }
}

function frameworkMetadata(value: unknown): FrameworkMetadata {
  const recordValue = record(value, "framework");
  return {
    name: requiredString(recordValue, "name", "framework.name"),
    version: requiredString(recordValue, "version", "framework.version"),
  };
}

function runtimeMetadata(value: unknown): RuntimeMetadata {
  const recordValue = record(value, "runtime");
  if (recordValue.engine !== "workerd") {
    fail("runtime.engine must be workerd");
  }
  const compatibilityDate = requiredString(
    recordValue,
    "compatibility_date",
    "runtime.compatibility_date",
  );
  if (!/^[0-9]{4}-[0-9]{2}-[0-9]{2}$/.test(compatibilityDate)) {
    fail("runtime.compatibility_date must be YYYY-MM-DD");
  }
  const flags = stringArray(recordValue.compatibility_flags, "runtime.compatibility_flags");
  const capabilities = stringArray(
    recordValue.required_capabilities,
    "runtime.required_capabilities",
  );
  for (const capability of capabilities) {
    if (!(SUPPORTED_CAPABILITIES as readonly string[]).includes(capability)) {
      fail(`unsupported required capability: ${capability}`);
    }
  }
  return {
    engine: "workerd",
    compatibility_date: compatibilityDate,
    compatibility_flags: flags,
    required_capabilities: capabilities as ArtifactCapability[],
  };
}

function entryMetadata(value: unknown): EntryMetadata {
  const recordValue = record(value, "entry");
  if (recordValue.type !== "worker") {
    fail("entry.type must be worker");
  }
  const path = requiredString(recordValue, "path", "entry.path");
  const exportName =
    recordValue.export === undefined
      ? "default"
      : requiredString(recordValue, "export", "entry.export");
  if (!/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(exportName)) {
    fail("entry.export is not a valid JavaScript identifier");
  }
  return { type: "worker", path, export: exportName };
}

function assetMetadata(value: unknown): AssetMetadata {
  const recordValue = record(value, "assets");
  return {
    directory: requiredString(recordValue, "directory", "assets.directory"),
    base_path: requiredString(recordValue, "base_path", "assets.base_path"),
  };
}

function renderingMetadata(value: unknown): RenderingMetadata {
  const recordValue = record(value, "rendering");
  if (!["static", "ssr", "hybrid"].includes(String(recordValue.mode))) {
    fail("rendering.mode must be static, ssr or hybrid");
  }
  if (typeof recordValue.streaming !== "boolean") {
    fail("rendering.streaming must be a boolean");
  }
  return {
    mode: recordValue.mode as RenderingMetadata["mode"],
    streaming: recordValue.streaming,
  };
}

function buildMetadata(value: unknown): BuildMetadata {
  const recordValue = record(value, "build");
  const createdAt = requiredString(recordValue, "created_at", "build.created_at");
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/.test(createdAt)) {
    fail("build.created_at must be an RFC 3339 timestamp");
  }
  return {
    build_id: requiredString(recordValue, "build_id", "build.build_id"),
    created_at: createdAt,
  };
}

function record(value: unknown, field: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    fail(`${field} must be an object`);
  }
  return value as Record<string, unknown>;
}

function requiredString(value: Record<string, unknown>, key: string, field: string): string {
  const candidate = value[key];
  if (typeof candidate !== "string" || candidate.length === 0) {
    fail(`${field} must be a non-empty string`);
  }
  return candidate;
}

function stringField(value: Record<string, unknown>, key: string): string {
  const candidate = value[key];
  if (typeof candidate !== "string") {
    fail(`${key} must be a string`);
  }
  return candidate;
}

function stringArray(value: unknown, field: string): string[] {
  if (value === undefined) {
    return [];
  }
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    fail(`${field} must be an array of strings`);
  }
  if (new Set(value).size !== value.length) {
    fail(`${field} must not contain duplicates`);
  }
  return value as string[];
}

async function assertDirectory(path: string, field: string): Promise<void> {
  const metadata = await stat(path).catch(() => null);
  if (!metadata?.isDirectory()) {
    fail(`${field} is not a directory: ${path}`);
  }
}

async function exists(path: string): Promise<boolean> {
  return (await lstat(path).catch(() => null)) !== null;
}

function fail(message: string): never {
  throw new ArtifactBuildError(message);
}
