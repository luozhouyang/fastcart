import { createApiClient, type ApiClientOptions } from "@fastcart/api-client";
import type { Money, PageResult } from "@fastcart/shared-types";

export interface AdminProductSummary {
  id: string;
  title: string;
  handle: string;
  status: "draft" | "active" | "archived";
}

export interface AdminOrderSummary {
  id: string;
  status: string;
  total: Money;
}

export interface AdminClient {
  products: {
    list(): Promise<PageResult<AdminProductSummary>>;
    get(id: string): Promise<AdminProductSummary>;
  };
  orders: {
    list(): Promise<PageResult<AdminOrderSummary>>;
    get(id: string): Promise<AdminOrderSummary>;
  };
}

export function createAdminClient(options: ApiClientOptions = {}): AdminClient {
  const client = createApiClient(options);

  return {
    products: {
      list: () => client.request<PageResult<AdminProductSummary>>("/api/v1/admin/products"),
      get: (id) => client.request<AdminProductSummary>(`/api/v1/admin/products/${id}`),
    },
    orders: {
      list: () => client.request<PageResult<AdminOrderSummary>>("/api/v1/admin/orders"),
      get: (id) => client.request<AdminOrderSummary>(`/api/v1/admin/orders/${id}`),
    },
  };
}
