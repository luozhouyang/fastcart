import assert from "node:assert/strict";
import { test } from "node:test";

import { PluginValidationError, validateManifest, type PluginManifest } from "../src/manifest.ts";

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
