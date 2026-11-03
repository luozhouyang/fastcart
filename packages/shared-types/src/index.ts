export interface StoreContext {
  storeId: string;
  domain: string;
  locale: string;
  currency: string;
  timezone: string;
}

export interface RequestContext {
  requestId: string;
  store?: StoreContext;
  actorId?: string;
}

export interface Money {
  amountMinor: number;
  currency: string;
}

export interface Pagination {
  limit: number;
  offset: number;
  total: number;
}

export interface PageResult<T> {
  data: T[];
  pagination: Pagination;
}

export interface ApiErrorBody {
  code: string;
  message: string;
  requestId?: string | null;
}

export interface ApiErrorResponse {
  error: ApiErrorBody;
}
