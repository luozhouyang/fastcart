import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

// The component module registers a custom element and touches DOM globals, so
// this test validates the shipped contract without a browser: the manifest
// entry resolves to a file, the module is an ES module, and the rendered
// markup uses user-controlled values as text rather than HTML.

const root = new URL("../", import.meta.url);

test("the manifest component entry ships as an ES module", async () => {
  const manifest = JSON.parse(await readFile(new URL("plugin.json", root), "utf8"));
  const component = manifest.provides.find(
    (contribution) => contribution.kind === "storefront_component",
  );
  assert.ok(component, "manifest declares a storefront component");
  assert.equal(component.id, "livechat-button");
  assert.equal(component.entry, "storefront/livechat-button.js");
  assert.deepEqual(component.requires, [{ id: "livechat", version: 1, optional: true }]);
  assert.ok(
    manifest.capabilities.storefront.includes("storefront.component"),
    "component contributions require the storefront.component scope",
  );

  const source = await readFile(new URL(component.entry, root), "utf8");
  assert.match(source, /customElements\.define\("fc-livechat-button"/);
  assert.match(source, /export \{/);
  assert.match(source, /\/store\/capabilities\/livechat/);
});

test("the component keeps capability values out of innerHTML", async () => {
  const source = await readFile(new URL("storefront/livechat-button.js", root), "utf8");
  // The only innerHTML assignment must be the static template literal; any
  // other assignment could reinterpret a server-provided greeting or accent.
  assert.doesNotMatch(source, /innerHTML\s*=(?!\s*`)/);
  assert.match(source, /textContent = nonEmptyString\(this\.getAttribute\("label"\)\)/);
  assert.match(source, /greetingNode\.textContent = this\.greeting/);
});
