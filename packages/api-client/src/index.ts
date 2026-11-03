import type {
  ApiErrorBody,
  ApiErrorResponse,
  PageResult,
  PaginationQuery,
} from "@fastcart/shared-types";

export type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

/** A token, or a function resolving the current token for each request. */
export type TokenSource = string | (() => string | Promise<string>);

/** Bearer authentication configuration. */
export interface ApiAuthOptions {
  /** Token source consulted for every request. */
  token: TokenSource;
  /** Authorization scheme; defaults to `Bearer`. */
  scheme?: string;
}

export interface ApiClientOptions {
  /** API origin, optionally including a path prefix. */
  baseUrl?: string;
  /** Fetch implementation; defaults to the global `fetch`. */
  fetch?: FetchLike;
  /** Headers merged into every request. */
  headers?: HeadersInit;
  /** Credentials mode forwarded to fetch, for example `include` for cookies. */
  credentials?: RequestCredentials;
  /** Bearer authentication configuration. */
  auth?: ApiAuthOptions;
  /** Static or dynamic correlation identifier sent as `x-request-id`. */
  requestId?: string | (() => string);
}

/**
 * A scalar or list query value. Arrays are serialized as repeated keys and
 * booleans as `true`/`false`. `null` and `undefined` entries are omitted.
 */
export type QueryValue =
  string | number | boolean | null | undefined | readonly (string | number | boolean)[];

export interface RequestOptions {
  method?: string;
  query?: Record<string, QueryValue>;
  body?: unknown;
  headers?: HeadersInit;
  signal?: AbortSignal;
  credentials?: RequestCredentials;
}

/** Options accepted by typed SDK method wrappers. */
export interface CallOptions {
  signal?: AbortSignal;
  headers?: HeadersInit;
}

export interface ApiClient {
  request<T>(path: string, options?: RequestOptions): Promise<T>;
  /** Performs the request and discards an empty or non-empty response body. */
  requestVoid(path: string, options?: RequestOptions): Promise<void>;
}

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly requestId: string | null;
  /** Parsed response body, when one was available. */
  readonly body: unknown;

  constructor(
    status: number,
    code: string,
    message: string,
    requestId: string | null,
    body?: unknown,
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.requestId = requestId;
    this.body = body;
  }
}

/** Resolves a token source, awaiting async providers. */
async function resolveToken(source: TokenSource): Promise<string> {
  return typeof source === "function" ? await source() : source;
}

/** Applies query parameters, preserving arrays as repeated keys. */
function appendQuery(search: URLSearchParams, query: Record<string, QueryValue>): void {
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === null) {
      continue;
    }
    if (Array.isArray(value)) {
      for (const item of value) {
        search.append(key, String(item));
      }
      continue;
    }
    search.set(key, String(value));
  }
}

/** Extracts the correlation identifier from an error envelope or header. */
export function responseRequestId(
  payload: ApiErrorResponse | undefined,
  response: Response,
): string | null {
  const body = payload?.error;
  return body?.request_id ?? body?.requestId ?? response.headers.get("x-request-id");
}

export function createApiClient(options: ApiClientOptions = {}): ApiClient {
  const baseUrl = options.baseUrl?.replace(/\/$/, "") ?? "";
  const doFetch: FetchLike = options.fetch ?? ((input, init) => globalThis.fetch(input, init));

  async function request<T>(path: string, requestOptions: RequestOptions = {}): Promise<T> {
    const query = new URLSearchParams();
    appendQuery(query, requestOptions.query ?? {});
    const search = query.toString();
    const separator = search.length === 0 ? "" : path.includes("?") ? "&" : "?";
    const url = `${baseUrl}${path}${separator}${search}`;

    const headers = new Headers(options.headers);
    if (options.auth !== undefined) {
      headers.set(
        "authorization",
        `${options.auth.scheme ?? "Bearer"} ${await resolveToken(options.auth.token)}`,
      );
    }
    if (options.requestId !== undefined && !headers.has("x-request-id")) {
      const requestId =
        typeof options.requestId === "function" ? options.requestId() : options.requestId;
      headers.set("x-request-id", requestId);
    }
    for (const [key, value] of new Headers(requestOptions.headers)) {
      headers.set(key, value);
    }

    let body: string | undefined;
    if (requestOptions.body !== undefined) {
      headers.set("content-type", "application/json");
      body = JSON.stringify(requestOptions.body);
    }

    const credentials = requestOptions.credentials ?? options.credentials;
    const response = await doFetch(url, {
      ...(requestOptions.method === undefined ? {} : { method: requestOptions.method }),
      headers,
      ...(body === undefined ? {} : { body }),
      ...(requestOptions.signal === undefined ? {} : { signal: requestOptions.signal }),
      ...(credentials === undefined ? {} : { credentials }),
    });

    if (!response.ok) {
      let code = "HTTP_ERROR";
      let message = response.statusText || `request failed with status ${response.status}`;
      let payload: ApiErrorResponse | undefined;
      try {
        payload = (await response.json()) as ApiErrorResponse;
        const error: ApiErrorBody | undefined = payload.error;
        if (error !== undefined) {
          code = error.code;
          message = error.message;
        }
      } catch {
        // Non-JSON error bodies fall back to the HTTP status message.
      }
      throw new ApiError(
        response.status,
        code,
        message,
        responseRequestId(payload, response),
        payload,
      );
    }

    if (response.status === 204 || response.headers.get("content-length") === "0") {
      return undefined as T;
    }

    const text = await response.text();
    if (text.length === 0) {
      return undefined as T;
    }
    return JSON.parse(text) as T;
  }

  async function requestVoid(path: string, requestOptions: RequestOptions = {}): Promise<void> {
    await request<unknown>(path, requestOptions);
  }

  return { request, requestVoid };
}

/** Normalizes a pagination query for use with `RequestOptions.query`. */
export function paginationQuery(query: PaginationQuery = {}): Record<string, number | undefined> {
  return { limit: query.limit, offset: query.offset };
}

/**
 * Iterates every record of a paginated endpoint.
 *
 * The callback receives the query for each page; iteration stops when a page is
 * empty or the reported total has been reached.
 */
export async function* paginate<T>(
  fetchPage: (query: PaginationQuery) => Promise<PageResult<T>>,
  options: PaginationQuery = {},
): AsyncGenerator<T, void, void> {
  const limit = options.limit ?? 20;
  let offset = options.offset ?? 0;
  for (;;) {
    const page = await fetchPage({ limit, offset });
    for (const item of page.data) {
      yield item;
    }
    offset += page.data.length;
    if (page.data.length === 0 || offset >= page.pagination.total) {
      return;
    }
  }
}
