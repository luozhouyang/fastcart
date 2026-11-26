import type {
  Category,
  Collection,
  CreateCategoryRequest,
  CreateCollectionRequest,
  CreateProductTypeRequest,
  PageResult,
  Product,
  ProductType,
  UpdateCategoryRequest,
  UpdateCollectionRequest,
  UpdateProductTypeRequest,
} from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** A category together with its nested children, ordered by rank then title. */
export interface CategoryNode {
  category: Category;
  children: CategoryNode[];
  /** Nesting depth; root categories are `0`. */
  depth: number;
}

/** An option for the parent picker: flattened tree order with a depth label. */
export interface CategoryParentOption {
  id: string;
  title: string;
  depth: number;
}

/** Largest page the taxonomy endpoints accept. */
const PAGE_LIMIT = 100;

/** Handle validation copied from the API so obvious typos fail before a request. */
const HANDLE_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

async function taxonomyRequest<T>(
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
    throw new Error(apiErrorMessage(body, `Catalog request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Reads every page of a list endpoint, capped for safety. */
async function fetchEvery<T>(
  fetchPage: (limit: number, offset: number) => Promise<PageResult<T>>,
  max = 2000,
): Promise<T[]> {
  const records: T[] = [];
  let offset = 0;
  while (records.length < max) {
    const page = await fetchPage(Math.min(PAGE_LIMIT, Math.max(1, max - records.length)), offset);
    records.push(...page.data);
    offset += page.data.length;
    if (page.data.length === 0 || offset >= page.pagination.total) break;
  }
  return records;
}

/* ------------------------------------------------------------------ */
/*  Categories                                                         */
/* ------------------------------------------------------------------ */

/** Lists every category in the store. */
export function listCategories(storeId: string, signal?: AbortSignal): Promise<Category[]> {
  return fetchEvery((limit, offset) =>
    taxonomyRequest<PageResult<Category>>(
      storeId,
      `/catalog/categories?limit=${limit}&offset=${offset}`,
      {
        ...(signal ? { signal } : {}),
      },
    ),
  );
}

/** Loads the direct children of one category. */
export function listChildCategories(
  storeId: string,
  parentId: string,
  signal?: AbortSignal,
): Promise<Category[]> {
  const params = new URLSearchParams({ parent_id: parentId, limit: String(PAGE_LIMIT) });
  return taxonomyRequest<PageResult<Category>>(
    storeId,
    `/catalog/categories?${params.toString()}`,
    signal ? { signal } : {},
  ).then((page) => page.data);
}

/** Creates a category. An omitted handle is derived from the title by the API. */
export function createCategory(
  storeId: string,
  input: CreateCategoryRequest,
  signal?: AbortSignal,
): Promise<Category> {
  return taxonomyRequest<Category>(storeId, "/catalog/categories", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Partially updates a category. Sending `parent_id: ""` clears the parent. */
export function updateCategory(
  storeId: string,
  categoryId: string,
  input: UpdateCategoryRequest,
  signal?: AbortSignal,
): Promise<Category> {
  return taxonomyRequest<Category>(
    storeId,
    `/catalog/categories/${encodeURIComponent(categoryId)}`,
    {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      ...(signal ? { signal } : {}),
    },
  );
}

/** Deletes a category. The API rejects the request when products still reference it. */
export function deleteCategory(
  storeId: string,
  categoryId: string,
  signal?: AbortSignal,
): Promise<void> {
  return taxonomyRequest<void>(storeId, `/catalog/categories/${encodeURIComponent(categoryId)}`, {
    method: "DELETE",
    ...(signal ? { signal } : {}),
  });
}

/* ------------------------------------------------------------------ */
/*  Collections                                                        */
/* ------------------------------------------------------------------ */

/** Lists every collection in the store. */
export function listCollections(storeId: string, signal?: AbortSignal): Promise<Collection[]> {
  return fetchEvery((limit, offset) =>
    taxonomyRequest<PageResult<Collection>>(
      storeId,
      `/catalog/collections?limit=${limit}&offset=${offset}`,
      { ...(signal ? { signal } : {}) },
    ),
  );
}

/** Creates a collection. */
export function createCollection(
  storeId: string,
  input: CreateCollectionRequest,
  signal?: AbortSignal,
): Promise<Collection> {
  return taxonomyRequest<Collection>(storeId, "/catalog/collections", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Partially updates a collection, including its publish state. */
export function updateCollection(
  storeId: string,
  collectionId: string,
  input: UpdateCollectionRequest,
  signal?: AbortSignal,
): Promise<Collection> {
  return taxonomyRequest<Collection>(
    storeId,
    `/catalog/collections/${encodeURIComponent(collectionId)}`,
    {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      ...(signal ? { signal } : {}),
    },
  );
}

/** Deletes a collection. */
export function deleteCollection(
  storeId: string,
  collectionId: string,
  signal?: AbortSignal,
): Promise<void> {
  return taxonomyRequest<void>(
    storeId,
    `/catalog/collections/${encodeURIComponent(collectionId)}`,
    {
      method: "DELETE",
      ...(signal ? { signal } : {}),
    },
  );
}

/** Publishes a collection so the storefront can serve it. */
export function publishCollection(
  storeId: string,
  collectionId: string,
  signal?: AbortSignal,
): Promise<Collection> {
  return taxonomyRequest<Collection>(
    storeId,
    `/catalog/collections/${encodeURIComponent(collectionId)}/publish`,
    {
      method: "POST",
      ...(signal ? { signal } : {}),
    },
  );
}

/** Unpublishes a collection. */
export function unpublishCollection(
  storeId: string,
  collectionId: string,
  signal?: AbortSignal,
): Promise<Collection> {
  return taxonomyRequest<Collection>(
    storeId,
    `/catalog/collections/${encodeURIComponent(collectionId)}/unpublish`,
    {
      method: "POST",
      ...(signal ? { signal } : {}),
    },
  );
}

/** Adds one or more products to a collection. */
export function addProductsToCollection(
  storeId: string,
  collectionId: string,
  productIds: string[],
  signal?: AbortSignal,
): Promise<void> {
  return taxonomyRequest<void>(
    storeId,
    `/catalog/collections/${encodeURIComponent(collectionId)}/products`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ product_ids: productIds }),
      ...(signal ? { signal } : {}),
    },
  );
}

/** Removes one product from a collection. */
export function removeProductFromCollection(
  storeId: string,
  collectionId: string,
  productId: string,
  signal?: AbortSignal,
): Promise<void> {
  return taxonomyRequest<void>(
    storeId,
    `/catalog/collections/${encodeURIComponent(collectionId)}/products/${encodeURIComponent(productId)}`,
    {
      method: "DELETE",
      ...(signal ? { signal } : {}),
    },
  );
}

/** Lists the products currently linked to a collection. */
export function listCollectionProducts(
  storeId: string,
  collectionId: string,
  signal?: AbortSignal,
): Promise<Product[]> {
  return fetchEvery((limit, offset) =>
    taxonomyRequest<PageResult<Product>>(
      storeId,
      `/catalog/products?collection_id=${encodeURIComponent(collectionId)}&limit=${limit}&offset=${offset}`,
      { ...(signal ? { signal } : {}) },
    ),
  );
}

/** Searches products by title, handle or SKU for the membership picker. */
export async function searchProducts(
  storeId: string,
  query: string,
  signal?: AbortSignal,
): Promise<Product[]> {
  const params = new URLSearchParams({ limit: "8" });
  const trimmed = query.trim();
  if (trimmed.length > 0) params.set("query", trimmed);
  const page = await taxonomyRequest<PageResult<Product>>(
    storeId,
    `/catalog/products?${params.toString()}`,
    signal ? { signal } : {},
  );
  return page.data;
}

/* ------------------------------------------------------------------ */
/*  Product types                                                      */
/* ------------------------------------------------------------------ */

/** Lists every product type in the store. */
export function listProductTypes(storeId: string, signal?: AbortSignal): Promise<ProductType[]> {
  return fetchEvery((limit, offset) =>
    taxonomyRequest<PageResult<ProductType>>(
      storeId,
      `/catalog/product-types?limit=${limit}&offset=${offset}`,
      { ...(signal ? { signal } : {}) },
    ),
  );
}

/** Creates a product type. The API rejects duplicates case-insensitively. */
export function createProductType(
  storeId: string,
  input: CreateProductTypeRequest,
  signal?: AbortSignal,
): Promise<ProductType> {
  return taxonomyRequest<ProductType>(storeId, "/catalog/product-types", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Renames a product type. */
export function updateProductType(
  storeId: string,
  productTypeId: string,
  input: UpdateProductTypeRequest,
  signal?: AbortSignal,
): Promise<ProductType> {
  return taxonomyRequest<ProductType>(
    storeId,
    `/catalog/product-types/${encodeURIComponent(productTypeId)}`,
    {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      ...(signal ? { signal } : {}),
    },
  );
}

/** Deletes a product type and clears it from every product that used it. */
export function deleteProductType(
  storeId: string,
  productTypeId: string,
  signal?: AbortSignal,
): Promise<void> {
  return taxonomyRequest<void>(
    storeId,
    `/catalog/product-types/${encodeURIComponent(productTypeId)}`,
    {
      method: "DELETE",
      ...(signal ? { signal } : {}),
    },
  );
}

/* ------------------------------------------------------------------ */
/*  Pure helpers                                                       */
/* ------------------------------------------------------------------ */

function compareCategories(left: Category, right: Category): number {
  return (
    left.rank - right.rank ||
    left.title.localeCompare(right.title) ||
    left.id.localeCompare(right.id)
  );
}

/**
 * Nests categories by `parent_id`.
 *
 * Rows whose parent is missing from the input are treated as roots so a
 * partially loaded or stale page still renders instead of disappearing.
 */
export function buildCategoryTree(categories: readonly Category[]): CategoryNode[] {
  const byId = new Map(categories.map((category) => [category.id, category]));
  const childrenOf = new Map<string, Category[]>();
  const roots: Category[] = [];

  for (const category of categories) {
    const parentId =
      category.parent_id !== null && byId.has(category.parent_id) ? category.parent_id : null;
    if (parentId === null) {
      roots.push(category);
      continue;
    }
    const siblings = childrenOf.get(parentId) ?? [];
    siblings.push(category);
    childrenOf.set(parentId, siblings);
  }

  const orderedChildren = (parentId: string): Category[] =>
    [...(childrenOf.get(parentId) ?? [])].sort(compareCategories);

  const build = (category: Category, depth: number, seen: Set<string>): CategoryNode => {
    const nextSeen = new Set(seen);
    nextSeen.add(category.id);
    // Guard against a cycle that slipped past the API so rendering terminates.
    const children = orderedChildren(category.id)
      .filter((child) => !nextSeen.has(child.id))
      .map((child) => build(child, depth + 1, nextSeen));
    return { category, children, depth };
  };

  const rendered = new Set<string>();
  const nodes = [...roots].sort(compareCategories).map((category) => {
    const node = build(category, 0, new Set());
    for (const row of flattenCategoryTree([node])) rendered.add(row.category.id);
    return node;
  });

  // A parent cycle leaves no root behind; surface those rows as roots instead
  // of dropping them from the admin list.
  const unvisited = categories.filter((category) => !rendered.has(category.id));
  for (const category of unvisited.sort(compareCategories)) {
    if (rendered.has(category.id)) continue;
    const node = build(category, 0, new Set());
    for (const row of flattenCategoryTree([node])) rendered.add(row.category.id);
    nodes.push(node);
  }

  return nodes;
}

/** Flattens a tree back into pre-order rows for rendering. */
export function flattenCategoryTree(nodes: readonly CategoryNode[]): CategoryNode[] {
  const rows: CategoryNode[] = [];
  const visit = (node: CategoryNode) => {
    rows.push(node);
    node.children.forEach(visit);
  };
  nodes.forEach(visit);
  return rows;
}

/** Collects the ids of a category and every descendant. */
function subtreeIds(nodes: readonly CategoryNode[], rootId: string): Set<string> {
  const ids = new Set<string>();
  const visit = (node: CategoryNode) => {
    ids.add(node.category.id);
    node.children.forEach(visit);
  };
  for (const node of flattenCategoryTree(nodes)) {
    if (node.category.id === rootId) visit(node);
  }
  return ids;
}

/**
 * Builds the parent picker options for a category editor.
 *
 * The edited category and its descendants are excluded because the API
 * rejects cycles.
 */
export function categoryParentOptions(
  categories: readonly Category[],
  excludeId?: string,
): CategoryParentOption[] {
  const tree = buildCategoryTree(categories);
  const excluded = excludeId ? subtreeIds(tree, excludeId) : new Set<string>();
  return flattenCategoryTree(tree)
    .filter((node) => !excluded.has(node.category.id))
    .map((node) => ({ id: node.category.id, title: node.category.title, depth: node.depth }));
}

/** True when the handle matches the API's lowercase hyphenated shape. */
export function isValidHandle(handle: string): boolean {
  return HANDLE_PATTERN.test(handle.trim());
}

/** Validates an optional handle field; empty means "let the API derive it". */
export function handleError(raw: string): string | null {
  const handle = raw.trim();
  if (handle.length === 0) return null;
  if (handle.length > 120) return "Handles cannot be longer than 120 characters.";
  if (!HANDLE_PATTERN.test(handle)) {
    return "Use lowercase letters, digits and single hyphens.";
  }
  return null;
}

/** Counts collections by publish state for the tab summary. */
export function collectionCounts(collections: readonly Collection[]): {
  published: number;
  drafts: number;
} {
  const published = collections.filter((collection) => collection.is_published).length;
  return { published, drafts: collections.length - published };
}
