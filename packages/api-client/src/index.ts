import type { ApiErrorResponse } from "@fastcart/shared-types";

export type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export interface ApiClientOptions {
  baseUrl?: string;
  fetch?: FetchLike;
  headers?: HeadersInit;
}

export interface RequestOptions {
  method?: string;
  query?: Record<string, string | number | boolean | undefined>;
  body?: unknown;
  headers?: HeadersInit;
  signal?: AbortSignal;
}

export interface ApiClient {
  request<T>(path: string, options?: RequestOptions): Promise<T>;
}

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly requestId: string | null;

  constructor(status: number, code: string, message: string, requestId: string | null) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.requestId = requestId;
  }
}

export function createApiClient(options: ApiClientOptions = {}): ApiClient {
  const baseUrl = options.baseUrl?.replace(/\/$/, "") ?? "";
  const doFetch: FetchLike = options.fetch ?? ((input, init) => globalThis.fetch(input, init));

  async function request<T>(path: string, requestOptions: RequestOptions = {}): Promise<T> {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(requestOptions.query ?? {})) {
      if (value !== undefined) {
        query.set(key, String(value));
      }
    }

    const search = query.toString();
    const url = `${baseUrl}${path}${search.length > 0 ? `?${search}` : ""}`;
    const headers = new Headers(options.headers);
    for (const [key, value] of new Headers(requestOptions.headers)) {
      headers.set(key, value);
    }

    let body: string | undefined;
    if (requestOptions.body !== undefined) {
      headers.set("content-type", "application/json");
      body = JSON.stringify(requestOptions.body);
    }

    const response = await doFetch(url, {
      ...(requestOptions.method === undefined ? {} : { method: requestOptions.method }),
      headers,
      ...(body === undefined ? {} : { body }),
      ...(requestOptions.signal === undefined ? {} : { signal: requestOptions.signal }),
    });

    if (!response.ok) {
      let code = "HTTP_ERROR";
      let message = response.statusText || `request failed with status ${response.status}`;
      let requestId: string | null = null;
      try {
        const payload = (await response.json()) as ApiErrorResponse;
        code = payload.error.code;
        message = payload.error.message;
        requestId = payload.error.requestId ?? null;
      } catch {
        // Non-JSON error bodies fall back to the HTTP status message.
      }
      throw new ApiError(response.status, code, message, requestId);
    }

    if (response.status === 204) {
      return undefined as T;
    }

    return (await response.json()) as T;
  }

  return { request };
}
