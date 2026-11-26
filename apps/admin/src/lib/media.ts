import type { Asset } from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** Envelope returned by `GET /api/v1/admin/assets`. */
export interface AssetPage {
  data: Asset[];
  pagination: {
    limit: number;
    offset: number;
    total: number;
  };
}

/** Optional metadata accepted alongside the uploaded file. */
export interface UploadAssetFields {
  alt?: string;
  width?: number;
  height?: number;
}

/** Image types the API accepts (it detects the type from the bytes). */
export const IMAGE_CONTENT_TYPES = ["image/jpeg", "image/png", "image/webp", "image/gif"] as const;

/** Types that can be previewed inline in the grid. */
export function isPreviewable(asset: Pick<Asset, "content_type">): boolean {
  return (IMAGE_CONTENT_TYPES as readonly string[]).includes(asset.content_type);
}

async function assetRequest<T>(storeId: string, path: string, init: RequestInit = {}): Promise<T> {
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
    throw new Error(apiErrorMessage(body, `Media request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists one page of assets, optionally filtered by a filename or alt search. */
export function listAssets(
  storeId: string,
  query: { search?: string; limit?: number; offset?: number } = {},
  signal?: AbortSignal,
): Promise<AssetPage> {
  const params = new URLSearchParams();
  const search = query.search?.trim();
  if (search) params.set("search", search);
  params.set("limit", String(query.limit ?? 60));
  params.set("offset", String(query.offset ?? 0));
  return assetRequest<AssetPage>(storeId, `/assets?${params.toString()}`, signal ? { signal } : {});
}

/**
 * Uploads a file as multipart form data.
 *
 * The request omits `content-type` on purpose so the runtime sets the
 * multipart boundary, and the API detects the media type from the bytes.
 */
export function uploadAsset(
  storeId: string,
  file: File | Blob,
  fields: UploadAssetFields = {},
  signal?: AbortSignal,
): Promise<Asset> {
  const form = new FormData();
  const filename =
    typeof File !== "undefined" && file instanceof File && file.name.length > 0
      ? file.name
      : "upload";
  form.append("file", file, filename);
  if (fields.alt !== undefined && fields.alt.trim().length > 0) {
    form.append("alt", fields.alt.trim());
  }
  if (fields.width !== undefined) form.append("width", String(fields.width));
  if (fields.height !== undefined) form.append("height", String(fields.height));
  return assetRequest<Asset>(storeId, "/assets", {
    method: "POST",
    body: form,
    ...(signal ? { signal } : {}),
  });
}

/** Deletes an asset; the API soft-deletes metadata and purges the stored blob. */
export function deleteAsset(storeId: string, assetId: string, signal?: AbortSignal): Promise<void> {
  return assetRequest<void>(storeId, `/assets/${encodeURIComponent(assetId)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/**
 * Public asset URL.
 *
 * The API serves bytes from `/api/v1/store/assets/{id}` and resolves the store
 * from the request host, so the path stays relative in the browser.
 */
export function assetUrl(assetId: string): string {
  return `/api/v1/store/assets/${encodeURIComponent(assetId)}`;
}

/** Human-readable byte size, for example `2.4 MB`. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ["KB", "MB", "GB", "TB"] as const;
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const digits = value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}

/** Dimensions label such as `1600 × 900`, or null when unknown. */
export function dimensionLabel(asset: Pick<Asset, "width" | "height">): string | null {
  if (asset.width === null || asset.height === null) return null;
  return `${asset.width} × ${asset.height}`;
}

/** True when the browser reports a file type the API can store. */
export function isAcceptedFile(file: Pick<File, "type" | "name">): boolean {
  if ((IMAGE_CONTENT_TYPES as readonly string[]).includes(file.type)) return true;
  return /\.(jpe?g|png|webp|gif|mp4|pdf)$/i.test(file.name);
}
