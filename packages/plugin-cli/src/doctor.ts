/** Plugin diagnostics. */

import {
  missingReferences,
  readManifest,
  referencedFiles,
  sourceEntry,
  type PluginManifest,
} from "./manifest.ts";

export interface DoctorResult {
  manifest: PluginManifest;
  files: number;
  missing: string[];
}

/** Validates the manifest, entry source and referenced files. */
export async function doctorPlugin(root: string): Promise<DoctorResult> {
  const manifest = await readManifest(root);
  const references = referencedFiles(manifest);
  sourceEntry(root, manifest.entry.path);
  const missing = await missingReferences(root, manifest);
  const entryMissing = missing.includes(manifest.entry.path);
  if (entryMissing) {
    // A TypeScript entry may be present while the emitted .js path is not.
    missing.splice(missing.indexOf(manifest.entry.path), 1);
  }
  return { manifest, files: references.length - missing.length, missing };
}
