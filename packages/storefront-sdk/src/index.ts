import { createApiClient, type ApiClientOptions } from "@fastcart/api-client";
import type { Money, PageResult } from "@fastcart/shared-types";

export interface StoreProduct {
  id: string;
  handle: string;
  title: string;
  description: string;
  price: Money | null;
}

export interface StoreCollection {
  id: string;
  handle: string;
  title: string;
}

export interface CartLine {
  variantId: string;
  quantity: number;
}

export interface StoreCart {
  id: string;
  items: CartLine[];
}

export interface StorefrontClient {
  products: {
    list(): Promise<PageResult<StoreProduct>>;
    getByHandle(handle: string): Promise<StoreProduct>;
  };
  collections: {
    list(): Promise<PageResult<StoreCollection>>;
  };
  cart: {
    create(): Promise<StoreCart>;
    addItem(cartId: string, item: CartLine): Promise<StoreCart>;
  };
}

export function createStorefrontClient(options: ApiClientOptions = {}): StorefrontClient {
  const client = createApiClient(options);

  return {
    products: {
      list: () => client.request<PageResult<StoreProduct>>("/api/v1/store/products"),
      getByHandle: (handle) => client.request<StoreProduct>(`/api/v1/store/products/${handle}`),
    },
    collections: {
      list: () => client.request<PageResult<StoreCollection>>("/api/v1/store/collections"),
    },
    cart: {
      create: () => client.request<StoreCart>("/api/v1/store/carts", { method: "POST" }),
      addItem: (cartId, item) =>
        client.request<StoreCart>(`/api/v1/store/carts/${cartId}/items`, {
          method: "POST",
          body: item,
        }),
    },
  };
}
