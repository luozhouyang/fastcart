import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { PLUGIN_ROOT } from "./env.mjs";

const FIXTURES = path.join(PLUGIN_ROOT, "fixtures");

export function loadFixture(name) {
  const root = path.join(FIXTURES, name);
  const modules = {};
  walk(root, root, modules);
  const mainModule = modules["index.js"] !== undefined ? "index.js" : Object.keys(modules)[0];
  return { modules, mainModule };
}

function walk(root, directory, modules) {
  for (const entry of readdirSync(directory)) {
    const full = path.join(directory, entry);
    if (statSync(full).isDirectory()) {
      walk(root, full, modules);
      continue;
    }
    const relative = path.relative(root, full).replaceAll("\\", "/");
    const text = readFileSync(full, "utf8");
    modules[relative] = relative.endsWith(".json")
      ? { json: JSON.parse(text) }
      : relative.endsWith(".txt")
        ? { text }
        : text;
  }
}
