import assert from "node:assert/strict";
import test from "node:test";

import { CAPABILITY_HEADER, parseCapabilityGrants } from "../src/capabilities.ts";

function encodePayload(payload: string | Uint8Array): string {
  const bytes = typeof payload === "string" ? new TextEncoder().encode(payload) : payload;
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return `1.${btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "")}`;
}

function encodeEnvelope(envelope: unknown): string {
  return encodePayload(JSON.stringify(envelope));
}

test("the header name matches the gateway contract", () => {
  assert.equal(CAPABILITY_HEADER, "x-fastcart-capabilities");
});

test("absent or empty headers produce no grants", () => {
  assert.deepEqual(parseCapabilityGrants(null), {});
  assert.deepEqual(parseCapabilityGrants(undefined), {});
  assert.deepEqual(parseCapabilityGrants(""), {});
});

test("valid grants keep non-ASCII configuration", () => {
  const grants = parseCapabilityGrants(
    encodeEnvelope({
      v: 1,
      grants: {
        search: {
          version: 2,
          endpoint: "/store/capabilities/search",
          config: { title: "搜索", locale: "zh-CN" },
        },
        chat: {
          version: 1,
          endpoint: "/store/capabilities/chat",
          config: { greeting: "café ☕" },
        },
      },
    }),
  );

  assert.deepEqual(grants, {
    search: {
      version: 2,
      endpoint: "/store/capabilities/search",
      config: { title: "搜索", locale: "zh-CN" },
    },
    chat: {
      version: 1,
      endpoint: "/store/capabilities/chat",
      config: { greeting: "café ☕" },
    },
  });
});

test("a null config is a valid JSON value", () => {
  const grants = parseCapabilityGrants(
    encodeEnvelope({
      v: 1,
      grants: {
        search: { version: 1, endpoint: "/store/capabilities/search", config: null },
      },
    }),
  );
  assert.equal(grants.search?.config, null);
});

test("base64url '-' and '_' are restored before decoding", () => {
  const dashHeader = encodeEnvelope({
    v: 1,
    grants: { x: { version: 1, endpoint: "/e", config: "c€" } },
  });
  const underscoreHeader = encodeEnvelope({
    v: 1,
    grants: { x: { version: 1, endpoint: "/e", config: "c😀" } },
  });

  assert.ok(dashHeader.includes("-"), "fixture must contain a base64url dash");
  assert.ok(underscoreHeader.includes("_"), "fixture must contain a base64url underscore");
  assert.equal(parseCapabilityGrants(dashHeader).x?.config, "c€");
  assert.equal(parseCapabilityGrants(underscoreHeader).x?.config, "c😀");
});

test("unsupported version prefixes are rejected", () => {
  assert.throws(() => parseCapabilityGrants("2.e30"), /unsupported.*version prefix/i);
  assert.throws(() => parseCapabilityGrants("x.e30"), /unsupported.*version prefix/i);
  assert.throws(() => parseCapabilityGrants("e30"), /unsupported.*version prefix/i);
});

test("invalid base64url is rejected", () => {
  assert.throws(() => parseCapabilityGrants("1.!!!!"), /base64url/i);
  assert.throws(() => parseCapabilityGrants("1.a"), /base64url/i);
  assert.throws(() => parseCapabilityGrants("1.abc=def"), /base64url/i);
});

test("invalid UTF-8 is rejected", () => {
  assert.throws(() => parseCapabilityGrants(encodePayload(new Uint8Array([0xff]))), /UTF-8/i);
});

test("invalid JSON is rejected", () => {
  assert.throws(() => parseCapabilityGrants(encodePayload("not json")), /JSON/i);
});

test("the envelope must be an object with the supported version", () => {
  assert.throws(() => parseCapabilityGrants(encodeEnvelope(null)), /envelope/i);
  assert.throws(() => parseCapabilityGrants(encodeEnvelope([])), /envelope/i);
  assert.throws(() => parseCapabilityGrants(encodeEnvelope("payload")), /envelope/i);
  assert.throws(
    () => parseCapabilityGrants(encodeEnvelope({ v: 2, grants: {} })),
    /envelope version/i,
  );
  assert.throws(
    () => parseCapabilityGrants(encodeEnvelope({ v: "1", grants: {} })),
    /envelope version/i,
  );
  assert.throws(() => parseCapabilityGrants(encodeEnvelope({ grants: {} })), /envelope version/i);
});

test("grants must be an object", () => {
  assert.throws(
    () => parseCapabilityGrants(encodeEnvelope({ v: 1, grants: [] })),
    /"grants" must be a JSON object/i,
  );
  assert.throws(
    () => parseCapabilityGrants(encodeEnvelope({ v: 1, grants: null })),
    /"grants" must be a JSON object/i,
  );
  assert.throws(
    () => parseCapabilityGrants(encodeEnvelope({ v: 1, grants: "x" })),
    /"grants" must be a JSON object/i,
  );
  assert.throws(
    () => parseCapabilityGrants(encodeEnvelope({ v: 1 })),
    /"grants" must be a JSON object/i,
  );
});

test("capability ids follow the binding rules", () => {
  for (const id of ["Chat", "-chat", "chat-", "chat service", "a".repeat(121), ""]) {
    assert.throws(
      () =>
        parseCapabilityGrants(
          encodeEnvelope({
            v: 1,
            grants: {
              [id]: { version: 1, endpoint: "/store/capabilities/x", config: {} },
            },
          }),
        ),
      /invalid capability id/i,
      id,
    );
  }
});

test("grant versions must be positive integers", () => {
  for (const version of [0, -1, 1.5, "1", null, undefined]) {
    assert.throws(
      () =>
        parseCapabilityGrants(
          encodeEnvelope({
            v: 1,
            grants: {
              search: { version, endpoint: "/store/capabilities/search", config: {} },
            },
          }),
        ),
      /invalid version/i,
      String(version),
    );
  }
});

test("grant endpoints must be non-empty strings", () => {
  for (const endpoint of [undefined, null, "", 42]) {
    assert.throws(
      () =>
        parseCapabilityGrants(
          encodeEnvelope({
            v: 1,
            grants: {
              search: { version: 1, endpoint, config: {} },
            },
          }),
        ),
      /invalid endpoint/i,
      String(endpoint),
    );
  }
});

test("malformed grant objects are rejected", () => {
  for (const grant of [null, [], "grant", 1]) {
    assert.throws(
      () => parseCapabilityGrants(encodeEnvelope({ v: 1, grants: { search: grant } })),
      /must be a JSON object/i,
      JSON.stringify(grant),
    );
  }
});

test("grants must carry a config value", () => {
  assert.throws(
    () =>
      parseCapabilityGrants(
        encodeEnvelope({
          v: 1,
          grants: { search: { version: 1, endpoint: "/store/capabilities/search" } },
        }),
      ),
    /missing config/i,
  );
});

test("oversized headers are rejected before decoding", () => {
  assert.throws(
    () => parseCapabilityGrants(`1.${"A".repeat(64 * 1024)}`),
    /exceeds|too large|maximum/i,
  );
});
