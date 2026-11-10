/** Template scaffolding. */

import { cp, mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join, resolve } from "node:path";

export type TemplateName = "minimal" | "admin" | "chat";

export interface InitOptions {
  dir: string;
  template?: TemplateName;
  force?: boolean;
}

export interface InitResult {
  dir: string;
  template: TemplateName;
  files: string[];
}

const TEMPLATES = fileURLToPath(new URL("../templates/", import.meta.url));
const TEXT_EXTENSIONS = new Set([".json", ".ts", ".js", ".mjs", ".html", ".css", ".md", ".txt"]);

/** Copies a starter template and substitutes the plugin id. */
export async function initPlugin(options: InitOptions): Promise<InitResult> {
  const template = options.template ?? "minimal";
  const target = resolve(options.dir);
  const existing = await readdir(target).catch(() => [] as string[]);
  if (existing.length > 0 && !options.force) {
    throw new Error(`directory is not empty: ${target} (use --force)`);
  }
  await mkdir(target, { recursive: true });
  const source = join(TEMPLATES, template);
  await cp(source, target, { recursive: true, force: true });
  const pluginId = target.split(/[\\/]/).filter(Boolean).at(-1) ?? "my-plugin";
  const id =
    pluginId
      .toLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 120) || "my-plugin";
  const files = await walk(target);
  for (const file of files) {
    if (!TEXT_EXTENSIONS.has(file.slice(file.lastIndexOf(".")))) continue;
    const path = join(target, file);
    const content = await readFile(path, "utf8");
    if (content.includes("__PLUGIN_ID__")) {
      await writeFile(path, content.replaceAll("__PLUGIN_ID__", id));
    }
  }
  return { dir: target, template, files };
}

async function walk(root: string, prefix = ""): Promise<string[]> {
  const output: string[] = [];
  const entries = await readdir(join(root, prefix), { withFileTypes: true });
  for (const entry of entries) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) output.push(...(await walk(root, path)));
    else if (entry.isFile()) output.push(path);
  }
  return output;
}
