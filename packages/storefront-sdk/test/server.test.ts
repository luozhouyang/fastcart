import assert from "node:assert/strict";
import test from "node:test";

import { createServerStorefrontClient } from "../src/server.ts";

test("server client rewrites paths and attaches the internal token", async () => {
  const seen: Array<{ url: string; authorization: string | null }> = [];
  const client = createServerStorefrontClient({
    origin: "https://shop.example.com",
    storeId: "store_01",
    internalToken: "internal-secret",
    fetch: async (input, init) => {
      const url = input instanceof URL ? input : new URL(String(input));
      seen.push({
        url: url.toString(),
        authorization: new Headers(init?.headers).get("authorization"),
      });
      return Response.json({ products: [], total: 0 });
    },
  });

  await client.products.list({ limit: 5 });
  assert.equal(seen.length, 1);
  assert.equal(
    seen[0]?.url,
    "https://shop.example.com/internal/ssr/v1/stores/store_01/products?limit=5",
  );
  assert.equal(seen[0]?.authorization, "Bearer internal-secret");
});

test("server client encodes store ids and requires a token", () => {
  assert.throws(() => {
    // The token is required by the type system and by the API middleware.
    createServerStorefrontClient({
      origin: "https://shop.example.com",
      storeId: "store/../other",
      internalToken: "",
    });
  }, /token|empty/i);
});
