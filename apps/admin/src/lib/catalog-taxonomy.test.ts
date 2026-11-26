import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Category, Collection } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  addProductsToCollection,
  buildCategoryTree,
  categoryParentOptions,
  collectionCounts,
  createCategory,
  createCollection,
  deleteCategory,
  deleteCollection,
  flattenCategoryTree,
  handleError,
  isValidHandle,
  listCategories,
  listCollectionProducts,
  listCollections,
  publishCollection,
  removeProductFromCollection,
  searchProducts,
  unpublishCollection,
  updateCategory,
} from "@/lib/catalog-taxonomy";
import { page } from "@/test/fixtures";

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

function requestAt(
  mock: { mock: { calls: unknown[][] } },
  index = 0,
): { url: string; init: RequestInit } {
  const call = mock.mock.calls[index] as [string, RequestInit] | undefined;
  if (!call) throw new Error("fetch was not called");
  return { url: call[0], init: call[1] };
}

function category(overrides: Partial<Category> = {}): Category {
  return {
    id: "cat_1",
    parent_id: null,
    title: "Tableware",
    handle: "tableware",
    description: null,
    is_active: true,
    is_internal: false,
    rank: 0,
    metadata: {},
    created_at: 1,
    updated_at: 2,
    ...overrides,
  };
}

function collection(overrides: Partial<Collection> = {}): Collection {
  return {
    id: "col_1",
    title: "Summer",
    handle: "summer",
    description: null,
    is_published: false,
    rank: 0,
    metadata: {},
    created_at: 1,
    updated_at: 2,
    ...overrides,
  };
}

beforeEach(() => {
  document.cookie = "fc_csrf=csrf-token";
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "fc_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT";
});

describe("category client", () => {
  it("lists categories with store and csrf headers", async () => {
    const mock = vi.fn(async () => jsonResponse(page([category()])));
    vi.stubGlobal("fetch", mock);

    await expect(listCategories("store-1")).resolves.toEqual([category()]);

    const { url, init } = requestAt(mock);
    expect(url).toContain(`${API_BASE_URL}/api/v1/admin/catalog/categories?`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("creates a category with the JSON body", async () => {
    const mock = vi.fn(async () => jsonResponse(category(), 201));
    vi.stubGlobal("fetch", mock);

    await createCategory("store-1", { title: "Tableware", parent_id: "cat_root" });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/catalog/categories`);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({
      title: "Tableware",
      parent_id: "cat_root",
    });
  });

  it("patches a category by its encoded id", async () => {
    const mock = vi.fn(async () => jsonResponse(category({ title: "Glassware" })));
    vi.stubGlobal("fetch", mock);

    await updateCategory("store-1", "cat 1", { title: "Glassware", parent_id: "" });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/catalog/categories/cat%201`);
    expect(init.method).toBe("PATCH");
    expect(JSON.parse(init.body as string)).toEqual({ title: "Glassware", parent_id: "" });
  });

  it("deletes a category and returns void", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await expect(deleteCategory("store-1", "cat_1")).resolves.toBeUndefined();

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/catalog/categories/cat_1`);
    expect(init.method).toBe("DELETE");
  });
});

describe("collection client", () => {
  it("lists collections as pages", async () => {
    const mock = vi.fn(async () => jsonResponse(page([collection()])));
    vi.stubGlobal("fetch", mock);

    await expect(listCollections("store-1")).resolves.toEqual([collection()]);

    const { url } = requestAt(mock);
    expect(url).toContain(`${API_BASE_URL}/api/v1/admin/catalog/collections?`);
  });

  it("creates a collection", async () => {
    const mock = vi.fn(async () => jsonResponse(collection(), 201));
    vi.stubGlobal("fetch", mock);

    await createCollection("store-1", { title: "Summer", is_published: true });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/catalog/collections`);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({ title: "Summer", is_published: true });
  });

  it("publishes and unpublishes collections", async () => {
    const mock = vi.fn(async () => jsonResponse(collection({ is_published: true })));
    vi.stubGlobal("fetch", mock);

    await publishCollection("store-1", "col_1");
    await unpublishCollection("store-1", "col_1");

    expect(requestAt(mock, 0).url).toBe(
      `${API_BASE_URL}/api/v1/admin/catalog/collections/col_1/publish`,
    );
    expect(requestAt(mock, 0).init.method).toBe("POST");
    expect(requestAt(mock, 1).url).toBe(
      `${API_BASE_URL}/api/v1/admin/catalog/collections/col_1/unpublish`,
    );
  });

  it("adds and removes products", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await addProductsToCollection("store-1", "col_1", ["prod_1", "prod_2"]);
    await removeProductFromCollection("store-1", "col_1", "prod_1");

    const add = requestAt(mock, 0);
    expect(add.url).toBe(`${API_BASE_URL}/api/v1/admin/catalog/collections/col_1/products`);
    expect(add.init.method).toBe("POST");
    expect(JSON.parse(add.init.body as string)).toEqual({ product_ids: ["prod_1", "prod_2"] });

    const remove = requestAt(mock, 1);
    expect(remove.url).toBe(
      `${API_BASE_URL}/api/v1/admin/catalog/collections/col_1/products/prod_1`,
    );
    expect(remove.init.method).toBe("DELETE");
  });

  it("lists collection products filtered by collection", async () => {
    const mock = vi.fn(async () => jsonResponse(page([])));
    vi.stubGlobal("fetch", mock);

    await listCollectionProducts("store-1", "col_1");

    expect(requestAt(mock).url).toContain("collection_id=col_1");
  });

  it("searches products for the picker", async () => {
    const mock = vi.fn(async () => jsonResponse(page([])));
    vi.stubGlobal("fetch", mock);

    await searchProducts("store-1", "carafe");

    const { url } = requestAt(mock);
    expect(url).toContain("/api/v1/admin/catalog/products?");
    expect(url).toContain("query=carafe");
    expect(url).toContain("limit=8");
  });

  it("deletes a collection", async () => {
    const mock = vi.fn(async () => jsonResponse(null, 204));
    vi.stubGlobal("fetch", mock);

    await deleteCollection("store-1", "col 1");

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/catalog/collections/col%201`);
    expect(init.method).toBe("DELETE");
  });

  it("surfaces the API error message", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { code: "not_found", message: "category not found" } }, 404),
    );
    vi.stubGlobal("fetch", mock);

    await expect(deleteCategory("store-1", "missing")).rejects.toThrow("category not found");
  });
});

describe("category tree helpers", () => {
  const rows: Category[] = [
    category({ id: "root-b", title: "Home", rank: 1 }),
    category({ id: "root-a", title: "Apparel", rank: 0 }),
    category({ id: "child-a2", title: "Knitwear", parent_id: "root-a", rank: 1 }),
    category({ id: "child-a1", title: "Shirts", parent_id: "root-a", rank: 0 }),
    category({ id: "grandchild", title: "Tees", parent_id: "child-a1" }),
    category({ id: "orphan", title: "Orphan", parent_id: "missing" }),
  ];

  it("nests categories by parent and sorts by rank then title", () => {
    const tree = buildCategoryTree(rows);

    expect(tree.map((node) => node.category.id)).toEqual(["root-a", "orphan", "root-b"]);
    expect(tree[0]?.children.map((node) => node.category.id)).toEqual(["child-a1", "child-a2"]);
    expect(tree[0]?.children[0]?.children[0]?.category.id).toBe("grandchild");
    expect(tree[0]?.children[0]?.children[0]?.depth).toBe(2);
  });

  it("flattens a tree in pre-order", () => {
    const flat = flattenCategoryTree(buildCategoryTree(rows));
    expect(flat.map((node) => node.category.id)).toEqual([
      "root-a",
      "child-a1",
      "grandchild",
      "child-a2",
      "orphan",
      "root-b",
    ]);
  });

  it("excludes the edited category and its descendants from parent options", () => {
    const options = categoryParentOptions(rows, "root-a");
    expect(options.map((option) => option.id)).toEqual(["orphan", "root-b"]);

    const childOptions = categoryParentOptions(rows, "child-a1");
    expect(childOptions.map((option) => option.id)).toEqual([
      "root-a",
      "child-a2",
      "orphan",
      "root-b",
    ]);
  });

  it("reports depth for indented option labels", () => {
    const options = categoryParentOptions(rows);
    expect(options.find((option) => option.id === "grandchild")?.depth).toBe(2);
  });

  it("terminates when a parent cycle slips through", () => {
    const cyclic = [
      category({ id: "a", title: "A", parent_id: "b" }),
      category({ id: "b", title: "B", parent_id: "a" }),
    ];
    const tree = buildCategoryTree(cyclic);
    const flat = flattenCategoryTree(tree);
    expect(flat.length).toBeLessThanOrEqual(2);
  });
});

describe("taxonomy validation and counts", () => {
  it("validates handles with the API shape", () => {
    expect(isValidHandle("summer-sale")).toBe(true);
    expect(isValidHandle("summer--sale")).toBe(false);
    expect(isValidHandle("Summer")).toBe(false);
    expect(isValidHandle("-lead")).toBe(false);
  });

  it("treats an empty handle as valid", () => {
    expect(handleError("")).toBeNull();
    expect(handleError("  ")).toBeNull();
    expect(handleError("bad handle")).toMatch(/lowercase/);
    expect(handleError("a".repeat(121))).toMatch(/120/);
  });

  it("counts published and draft collections", () => {
    expect(
      collectionCounts([
        collection({ id: "1", is_published: true }),
        collection({ id: "2", is_published: false }),
        collection({ id: "3", is_published: true }),
      ]),
    ).toEqual({ published: 2, drafts: 1 });
  });
});
