//! Server-only storefront client for the FastCart SSR commerce API.
//!
//! This client never resolves a store from the public `Host` header. The store
//! id is part of the request path and the internal bearer token is attached to
//! every request.

import type { FetchLike } from "@fastcart/api-client";

import {
  createStorefrontClient,
  type StorefrontClient,
  type StorefrontClientOptions,
} from "./index.ts";

/** Options accepted by {@link createServerStorefrontClient}. */
export interface ServerStorefrontClientOptions extends Omit<
  StorefrontClientOptions,
  "baseUrl" | "fetch" | "credentials"
> {
  /** Commerce API origin, for example `http://127.0.0.1:8080`. */
  origin: string;
  /** Store selected by the server, not by the browser. */
  storeId: string;
  /** Short-lived internal bearer token for this deployment. */
  internalToken: string;
  /** Optional fetch implementation for tests or custom transports. */
  fetch?: FetchLike;
}

/** Creates a client restricted to the internal server-to-server namespace. */
export function createServerStorefrontClient(
  options: ServerStorefrontClientOptions,
): StorefrontClient {
  if (options.storeId.trim().length === 0) {
    throw new Error("storeId is required");
  }
  if (options.internalToken.trim().length === 0) {
    throw new Error("internalToken is required");
  }
  const doFetch: FetchLike = options.fetch ?? ((input, init) => globalThis.fetch(input, init));
  const prefix = `/internal/ssr/v1/stores/${encodeURIComponent(options.storeId)}`;

  const serverFetch: FetchLike = (input, init) => {
    const url =
      input instanceof URL
        ? new URL(input)
        : new URL(typeof input === "string" ? input : input.url, options.origin);
    if (url.pathname.startsWith("/api/v1/store")) {
      url.pathname = `${prefix}${url.pathname.slice("/api/v1/store".length)}`;
    }
    const headers = new Headers(init?.headers);
    headers.set("authorization", `Bearer ${options.internalToken}`);
    return doFetch(url, { ...init, headers });
  };

  return createStorefrontClient({
    ...options,
    baseUrl: options.origin,
    fetch: serverFetch,
    credentials: "include",
  });
}
