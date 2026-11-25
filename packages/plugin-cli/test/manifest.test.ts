import assert from "node:assert/strict";
import { test } from "node:test";

import {
  PluginValidationError,
  referencedFiles,
  validateManifest,
  type PluginManifest,
} from "../src/manifest.ts";

function base(): PluginManifest {
  return {
    schema_version: "1",
    id: "demo",
    name: "Demo",
    version: "1.0.0",
    api_version: 2,
    runtime: { engine: "workerd", compatibility_date: "2026-11-01" },
    entry: { type: "worker", path: "worker/index.js" },
    extensions: { admin: [{ target: "admin.nav.section", label: "Demo" }] },
  };
}

test("accepts a valid manifest", () => {
  validateManifest(base());
});

test("validates secret scopes and RPC permissions", () => {
  const valid = base();
  valid.capabilities = { secrets: ["secrets.read"] };
  valid.rpc = ["conversations.list"];
  valid.rpc_permissions = { "conversations.list": "plugins.write" };
  validateManifest(valid);

  const badScope = base();
  badScope.capabilities = { secrets: ["secrets.write"] };
  assert.throws(() => validateManifest(badScope), PluginValidationError);

  const undeclared = base();
  undeclared.rpc = ["conversations.list"];
  undeclared.rpc_permissions = { "conversations.reply": "plugins.write" };
  assert.throws(() => validateManifest(undeclared), PluginValidationError);

  const badPermission = base();
  badPermission.rpc = ["conversations.list"];
  badPermission.rpc_permissions = { "conversations.list": "Plugins.Write" };
  assert.throws(() => validateManifest(badPermission), PluginValidationError);
});

test("rejects traversal, unknown targets and bad versions", () => {
  const traversal = base();
  traversal.entry.path = "../index.js";
  assert.throws(() => validateManifest(traversal), PluginValidationError);

  const target = base();
  target.extensions!.admin![0]!.target = "admin.nope";
  assert.throws(() => validateManifest(target), PluginValidationError);

  const version = base();
  version.version = "1.0";
  assert.throws(() => validateManifest(version), PluginValidationError);
});

function capability(manifest: PluginManifest) {
  return manifest.provides![0] as unknown as {
    id: string;
    version: number;
    config_schema?: unknown;
  };
}

function component(manifest: PluginManifest) {
  return manifest.provides![1] as unknown as {
    id: string;
    entry?: string;
    editor_manifest?: unknown;
    requires: Array<{ id: string; version?: number }>;
  };
}

function contributions(): PluginManifest {
  const manifest = base();
  manifest.capabilities = { storefront: ["store.read", "storefront.component"] };
  manifest.provides = [
    {
      kind: "storefront_capability",
      id: "search",
      version: 1,
      title: "Search",
      config_schema: { type: "object" },
    },
    {
      kind: "storefront_component",
      id: "hero-banner",
      title: "Hero banner",
      category: "marketing",
      editor_manifest: { fields: ["title"] },
      entry: "components/hero.js",
      requires: [{ id: "search", version: 1 }, { id: "recommendations", optional: true }],
    },
  ];
  return manifest;
}

test("accepts storefront contributions and references their entries", () => {
  const manifest = contributions();
  validateManifest(manifest);
  assert.deepEqual(referencedFiles(manifest), ["worker/index.js", "components/hero.js"]);
});

test("rejects duplicate and malformed contributions", () => {
  const duplicate = contributions();
  duplicate.provides!.push({
    kind: "storefront_capability",
    id: "search",
    version: 1,
    title: "Search again",
  });
  assert.throws(() => validateManifest(duplicate), PluginValidationError);

  const duplicateComponent = contributions();
  duplicateComponent.provides!.push({
    kind: "storefront_component",
    id: "hero-banner",
    title: "Hero again",
  });
  assert.throws(() => validateManifest(duplicateComponent), PluginValidationError);

  const badId = contributions();
  capability(badId).id = "Bad Id";
  assert.throws(() => validateManifest(badId), PluginValidationError);

  const badVersion = contributions();
  capability(badVersion).version = 0;
  assert.throws(() => validateManifest(badVersion), PluginValidationError);

  const fractionalVersion = contributions();
  capability(fractionalVersion).version = 1.5;
  assert.throws(() => validateManifest(fractionalVersion), PluginValidationError);

  const badRequirement = contributions();
  component(badRequirement).requires = [{ id: "search", version: 0 }];
  assert.throws(() => validateManifest(badRequirement), PluginValidationError);
});

test("rejects unsafe component entries and non-object manifests", () => {
  for (const entry of ["../hero.js", "/components/hero.js", "components/hero.css"]) {
    const manifest = contributions();
    component(manifest).entry = entry;
    assert.throws(() => validateManifest(manifest), PluginValidationError);
  }

  const schema = contributions();
  capability(schema).config_schema = "not-an-object";
  assert.throws(() => validateManifest(schema), PluginValidationError);

  const editor = contributions();
  component(editor).editor_manifest = [];
  assert.throws(() => validateManifest(editor), PluginValidationError);
});

test("rejects components without the storefront.component scope", () => {
  const manifest = contributions();
  manifest.capabilities = { storefront: ["store.read"] };
  assert.throws(() => validateManifest(manifest), PluginValidationError);
});
