import type {
  Translation,
  TranslationQuery,
  UpsertTranslationRequest,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** One field of a translated resource as the editor presents it. */
export interface TranslationField {
  /** Key used in the translation `fields` object. */
  key: string;
  /** Merchant-facing label. */
  label: string;
  /** Render a textarea instead of a single-line input. */
  multiline?: boolean;
}

/** Counts how much of one translation is filled in. */
export interface TranslationCompleteness {
  /** Fields with a non-blank translation. */
  translated: number;
  /** Fields the resource exposes for translation. */
  total: number;
  /** Field keys still missing a translation, in input order. */
  missing: string[];
}

/**
 * A failed translation request, carrying the HTTP status.
 *
 * The panel needs the status to tell "no translation for this locale yet"
 * (404) apart from a real failure.
 */
export class TranslationRequestError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "TranslationRequestError";
    this.status = status;
  }
}

/** True when the API reported that the translation row does not exist. */
export function isTranslationNotFound(cause: unknown): boolean {
  return cause instanceof TranslationRequestError && cause.status === 404;
}

async function translationRequest<T>(
  storeId: string,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("x-store-id", storeId);
  const csrf = readCookie("fc_csrf");
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/admin${path}`, {
    ...init,
    credentials: "include",
    headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new TranslationRequestError(
      apiErrorMessage(body, `Translation request failed (${response.status})`),
      response.status,
    );
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

function translationParams(query: TranslationQuery): string {
  const params = new URLSearchParams({
    resource_type: query.resource_type,
    resource_id: query.resource_id,
  });
  if (query.locale) params.set("locale", query.locale);
  return params.toString();
}

/**
 * Lists the translations of one resource.
 *
 * Without a `locale` the API returns every row; with one it returns just that
 * locale and answers 404 when no row exists yet.
 */
export function listTranslations(
  storeId: string,
  query: TranslationQuery,
  signal?: AbortSignal,
): Promise<Translation[]> {
  return translationRequest<Translation[]>(
    storeId,
    `/content/translations?${translationParams(query)}`,
    signal ? { signal } : {},
  );
}

/** Creates or replaces the translation of one resource and locale. */
export function upsertTranslation(
  storeId: string,
  query: TranslationQuery & { locale: string },
  input: UpsertTranslationRequest,
  signal?: AbortSignal,
): Promise<Translation> {
  return translationRequest<Translation>(
    storeId,
    `/content/translations?${translationParams(query)}`,
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      ...(signal ? { signal } : {}),
    },
  );
}

/** Deletes the translation of one resource and locale. */
export function deleteTranslation(
  storeId: string,
  query: TranslationQuery & { locale: string },
  signal?: AbortSignal,
): Promise<void> {
  return translationRequest<void>(storeId, `/content/translations?${translationParams(query)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/* ------------------------------------------------------------------ */
/*  Pure helpers                                                       */
/* ------------------------------------------------------------------ */

/** True when a stored field carries a non-blank translation. */
export function hasTranslatedValue(value: unknown): boolean {
  if (typeof value === "string") return value.trim().length > 0;
  return value !== null && value !== undefined;
}

/** Renders a stored translation or source value as editable text. */
export function translationText(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

/**
 * Overlays a translation onto the source content.
 *
 * Blank and missing translated fields keep the source value, matching the
 * storefront's per-field fallback.
 */
export function mergeTranslationFields(
  source: Record<string, unknown>,
  translation: Pick<Translation, "fields"> | null,
  keys?: readonly string[],
): Record<string, unknown> {
  const allowed = keys ? new Set(keys) : null;
  const merged: Record<string, unknown> = { ...source };
  for (const [key, value] of Object.entries(translation?.fields ?? {})) {
    if (allowed && !allowed.has(key)) continue;
    if (!hasTranslatedValue(value)) continue;
    merged[key] = value;
  }
  return merged;
}

/** Reports how many of the resource's fields the translation fills in. */
export function translationCompleteness(
  translation: Pick<Translation, "fields"> | null,
  keys: readonly string[],
): TranslationCompleteness {
  const fields = translation?.fields ?? {};
  const missing = keys.filter((key) => !hasTranslatedValue(fields[key]));
  return { translated: keys.length - missing.length, total: keys.length, missing };
}

/**
 * Collects the non-blank draft values into a translation `fields` object.
 *
 * Untranslated fields are omitted so the storefront falls back to the source
 * content for them.
 */
export function collectTranslationFields(
  values: Record<string, string>,
  keys: readonly string[],
): Record<string, unknown> {
  const fields: Record<string, unknown> = {};
  for (const key of keys) {
    const value = (values[key] ?? "").trim();
    if (value.length > 0) fields[key] = value;
  }
  return fields;
}

/** Stable key for one translation row, used to key demo state. */
export function translationKey(resourceType: string, resourceId: string, locale: string): string {
  return `${resourceType}\u0000${resourceId}\u0000${locale}`;
}
