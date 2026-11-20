/** Bundling and deterministic artifact creation. */

import { createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { build } from "esbuild";

import {
  PluginValidationError,
  missingReferences,
  readManifest,
  referencedFiles,
  sourceEntry,
  type PluginManifest,
} from "./manifest.ts";
import { createTarGz, type TarEntry } from "./tar.ts";

const EXCLUDED_DIRECTORIES = new Set(["node_modules", ".git", "dist", ".fastcart", "coverage"]);
const EXCLUDED_FILES = new Set([
  "package.json",
  "tsconfig.json",
  "pnpm-lock.yaml",
  "package-lock.json",
]);

export interface PackOptions {
  /** Plugin root containing plugin.json. */
  root: string;
  /** Output archive or directory. Defaults to `<root>/dist`. */
  out?: string;
  /** Disable esbuild and copy the entry as-is. */
  bundle?: boolean;
  /** Keep the staging directory for inspection. */
  keepStaging?: boolean;
}

export interface PackResult {
  archive: string;
  digest: string;
  bytes: number;
  files: number;
  manifest: PluginManifest;
}

/** Bundles, validates and writes a deterministic tar.gz artifact. */
export async function packPlugin(options: PackOptions): Promise<PackResult> {
  const root = resolve(options.root);
  const manifest = await readManifest(root);
  const bundle = options.bundle ?? true;
  const staging = await mkdtemp(join(tmpdir(), "fastcart-plugin-"));
  try {
    const source = bundle
      ? sourceEntry(root, manifest.entry.path)
      : join(root, manifest.entry.path);
    const entrySource = relative(root, source);
    await copyTree(root, staging, new Set([entrySource]));
    if (bundle) {
      await mkdir(dirname(join(staging, manifest.entry.path)), { recursive: true });
      await build({
        absWorkingDir: root,
        entryPoints: [source],
        outfile: join(staging, manifest.entry.path),
        bundle: true,
        format: "esm",
        platform: "neutral",
        target: "es2022",
        external: ["node:*"],
        logLevel: "silent",
        sourcemap: false,
        minify: false,
        legalComments: "none",
      });
    } else if (source !== join(staging, manifest.entry.path)) {
      await mkdir(dirname(join(staging, manifest.entry.path)), { recursive: true });
      await copyFile(source, join(staging, manifest.entry.path));
    }

    const missing = await missingReferences(staging, manifest);
    if (missing.length > 0) {
      throw new PluginValidationError(`missing referenced files: ${missing.join(", ")}`);
    }

    const files = (await collectFiles(staging)).sort((left, right) => left.localeCompare(right));
    const entries: TarEntry[] = [];
    for (const file of files) {
      entries.push({ path: file, data: await readFile(join(staging, file)) });
    }
    const gzip = createTarGz(entries);
    const digest = createHash("sha256").update(gzip).digest("hex");

    const outputDir = options.out ? resolve(options.out) : join(root, "dist");
    await mkdir(outputDir, { recursive: true });
    const archive = outputDir.endsWith(".tar.gz")
      ? outputDir
      : join(outputDir, `${manifest.id}-${manifest.version}.tar.gz`);
    await writeFile(archive, gzip);
    return { archive, digest, bytes: gzip.length, files: files.length, manifest };
  } finally {
    if (!options.keepStaging) await rm(staging, { recursive: true, force: true });
  }
}

/** Validates a plugin without writing an artifact. */
export async function doctorPlugin(
  root: string,
): Promise<{ manifest: PluginManifest; missing: string[] }> {
  const manifest = await readManifest(root);
  const missing = await missingReferences(root, manifest);
  sourceEntry(root, manifest.entry.path);
  return { manifest, missing };
}

async function copyTree(source: string, target: string, skip: Set<string>): Promise<void> {
  const entries = await readdir(source, { withFileTypes: true });
  await mkdir(target, { recursive: true });
  for (const entry of entries) {
    const relativePath = relative(source, join(source, entry.name));
    if (skip.has(relativePath)) continue;
    if (entry.isDirectory()) {
      if (EXCLUDED_DIRECTORIES.has(entry.name)) continue;
      await copyTree(join(source, entry.name), join(target, entry.name), skip);
      continue;
    }
    if (!entry.isFile()) continue;
    if (EXCLUDED_FILES.has(entry.name)) continue;
    if (entry.name.endsWith(".ts") || entry.name.endsWith(".tsx")) continue;
    await copyFile(join(source, entry.name), join(target, entry.name));
  }
}

async function collectFiles(root: string, prefix = ""): Promise<string[]> {
  const output: string[] = [];
  const entries = await readdir(join(root, prefix), { withFileTypes: true });
  for (const entry of entries) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) {
      output.push(...(await collectFiles(root, path)));
    } else if (entry.isFile()) {
      output.push(path);
    }
  }
  return output;
}

/** Returns the files the manifest promises, for doctor output. */
export async function expectedFiles(root: string): Promise<string[]> {
  const manifest = await readManifest(root);
  return referencedFiles(manifest);
}

/** Returns the total bytes of the plugin source tree, excluding build outputs. */
export async function sourceBytes(root: string): Promise<number> {
  const files = await collectFiles(root);
  let total = 0;
  for (const file of files) {
    const info = await stat(join(root, file)).catch(() => null);
    if (info) total += info.size;
  }
  return total;
}
