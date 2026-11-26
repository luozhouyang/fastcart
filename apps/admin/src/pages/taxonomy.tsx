import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FolderTree,
  Layers3,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";
import type {
  Category,
  Collection,
  CreateCategoryRequest,
  CreateCollectionRequest,
  Product,
  UpdateCategoryRequest,
  UpdateCollectionRequest,
} from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
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
  listCategories,
  listCollectionProducts,
  listCollections,
  publishCollection,
  removeProductFromCollection,
  searchProducts,
  unpublishCollection,
  updateCategory,
  updateCollection,
} from "@/lib/catalog-taxonomy";
import { demoProducts } from "@/lib/demo";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { useAsyncData } from "@/lib/use-async-data";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-60";

interface TaxonomyData {
  categories: Category[];
  collections: Collection[];
}

interface CategoryDraft {
  title: string;
  handle: string;
  parentId: string;
  description: string;
  isActive: boolean;
  isInternal: boolean;
}

interface CollectionDraft {
  title: string;
  handle: string;
  description: string;
  isPublished: boolean;
}

const DEMO_CATEGORY_FIXTURES: Category[] = [
  {
    id: "cat_demo_home",
    parent_id: null,
    title: "Home",
    handle: "home",
    description: "Objects for the table and the shelf.",
    is_active: true,
    is_internal: false,
    rank: 0,
    metadata: {},
    created_at: 1_759_800_000,
    updated_at: 1_759_886_400,
  },
  {
    id: "cat_demo_tableware",
    parent_id: "cat_demo_home",
    title: "Tableware",
    handle: "tableware",
    description: "Plates, bowls and carafes.",
    is_active: true,
    is_internal: false,
    rank: 0,
    metadata: {},
    created_at: 1_759_810_000,
    updated_at: 1_759_896_400,
  },
  {
    id: "cat_demo_drinkware",
    parent_id: "cat_demo_tableware",
    title: "Drinkware",
    handle: "drinkware",
    description: null,
    is_active: true,
    is_internal: false,
    rank: 1,
    metadata: {},
    created_at: 1_759_820_000,
    updated_at: 1_759_906_400,
  },
  {
    id: "cat_demo_lighting",
    parent_id: null,
    title: "Lighting",
    handle: "lighting",
    description: "Lamps and shades.",
    is_active: true,
    is_internal: false,
    rank: 1,
    metadata: {},
    created_at: 1_759_830_000,
    updated_at: 1_759_916_400,
  },
  {
    id: "cat_demo_archive",
    parent_id: null,
    title: "Prototypes",
    handle: "prototypes",
    description: "Internal-only samples.",
    is_active: false,
    is_internal: true,
    rank: 2,
    metadata: {},
    created_at: 1_759_840_000,
    updated_at: 1_759_926_400,
  },
];

const DEMO_COLLECTION_FIXTURES: Collection[] = [
  {
    id: "col_demo_new",
    title: "New arrivals",
    handle: "new-arrivals",
    description: "The latest additions to the catalog.",
    is_published: true,
    rank: 0,
    metadata: {},
    created_at: 1_760_000_000,
    updated_at: 1_760_086_400,
  },
  {
    id: "col_demo_sale",
    title: "Late summer sale",
    handle: "late-summer-sale",
    description: null,
    is_published: false,
    rank: 1,
    metadata: {},
    created_at: 1_760_100_000,
    updated_at: 1_760_186_400,
  },
];

/** Demo collection membership keyed by collection id. */
const DEMO_COLLECTION_MEMBERS: Record<string, string[]> = {
  col_demo_new: ["p-01", "p-02"],
  col_demo_sale: [],
};

function demoCategoryRows(): Category[] {
  return DEMO_CATEGORY_FIXTURES.map((category) => ({ ...category }));
}

function demoCollectionRows(): Collection[] {
  return DEMO_COLLECTION_FIXTURES.map((collection) => ({ ...collection }));
}

export function TaxonomyPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [tab, setTab] = useState("categories");
  const [demoCategories, setDemoCategories] = useState<Category[]>(demoCategoryRows);
  const [demoCollections, setDemoCollections] = useState<Collection[]>(demoCollectionRows);
  const [demoMembers, setDemoMembers] = useState<Record<string, string[]>>(DEMO_COLLECTION_MEMBERS);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [categorySearch, setCategorySearch] = useState("");
  const [collectionSearch, setCollectionSearch] = useState("");

  const [categoryDialogOpen, setCategoryDialogOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<Category | null>(null);
  const [deleteCategoryTarget, setDeleteCategoryTarget] = useState<Category | null>(null);
  const [categoryBusy, setCategoryBusy] = useState(false);

  const [collectionDialogOpen, setCollectionDialogOpen] = useState(false);
  const [editingCollection, setEditingCollection] = useState<Collection | null>(null);
  const [deleteCollectionTarget, setDeleteCollectionTarget] = useState<Collection | null>(null);
  const [collectionBusy, setCollectionBusy] = useState(false);

  /** Parent preselected when "Add subcategory" opens the dialog. */
  const [pendingParentId, setPendingParentId] = useState("");

  const [membersCollection, setMembersCollection] = useState<Collection | null>(null);
  const [memberProducts, setMemberProducts] = useState<Product[]>([]);
  const [memberLoading, setMemberLoading] = useState(false);
  const [memberQuery, setMemberQuery] = useState("");
  const [memberResults, setMemberResults] = useState<Product[]>([]);
  const [memberSearching, setMemberSearching] = useState(false);
  const [memberBusyId, setMemberBusyId] = useState<string | null>(null);

  useEffect(() => {
    setCategoryDialogOpen(false);
    setEditingCategory(null);
    setDeleteCategoryTarget(null);
    setCollectionDialogOpen(false);
    setEditingCollection(null);
    setDeleteCollectionTarget(null);
    setMembersCollection(null);
    setExpanded(new Set());
    setCategorySearch("");
    setCollectionSearch("");
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<TaxonomyData> => {
      if (mode === "demo") {
        return Promise.resolve({ categories: demoCategories, collections: demoCollections });
      }
      if (!store) return Promise.reject(new Error("No store is selected."));
      return Promise.all([
        listCategories(store.id, signal),
        listCollections(store.id, signal),
      ]).then(([categories, collections]) => ({ categories, collections }));
    },
    [mode, store, demoCategories, demoCollections],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoCategories,
    demoCollections,
  ]);

  const categories = useMemo(() => data?.categories ?? [], [data]);
  const collections = useMemo(() => data?.collections ?? [], [data]);

  const categoryTree = useMemo(() => buildCategoryTree(categories), [categories]);
  const categoryRows = useMemo(() => flattenCategoryTree(categoryTree), [categoryTree]);

  const visibleCategoryRows = useMemo(() => {
    const needle = categorySearch.trim().toLowerCase();
    if (needle.length === 0) {
      return categoryRows.filter((node) => {
        // Hide rows whose ancestor is collapsed.
        const ancestors = ancestorIds(categories, node.category.id);
        return ancestors.every((id) => expanded.has(id)) || node.depth === 0;
      });
    }
    return categoryRows.filter(
      (node) =>
        node.category.title.toLowerCase().includes(needle) ||
        node.category.handle.toLowerCase().includes(needle),
    );
  }, [categoryRows, categorySearch, categories, expanded]);

  const filteredCollections = useMemo(() => {
    const needle = collectionSearch.trim().toLowerCase();
    const list = [...collections].sort(
      (left, right) => left.rank - right.rank || left.title.localeCompare(right.title),
    );
    if (needle.length === 0) return list;
    return list.filter(
      (collection) =>
        collection.title.toLowerCase().includes(needle) ||
        collection.handle.toLowerCase().includes(needle),
    );
  }, [collections, collectionSearch]);

  useEffect(() => {
    if (!membersCollection || mode !== "api" || !store) return;
    const controller = new AbortController();
    setMemberLoading(true);
    Promise.all([
      listCollectionProducts(store.id, membersCollection.id, controller.signal),
      searchProducts(store.id, "", controller.signal),
    ])
      .then(([members, results]) => {
        setMemberProducts(members);
        setMemberResults(results);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) toast.error(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setMemberLoading(false);
      });
    return () => controller.abort();
  }, [membersCollection, mode, store]);

  useEffect(() => {
    if (!membersCollection || mode !== "api" || !store) return;
    const controller = new AbortController();
    const handle = window.setTimeout(() => {
      setMemberSearching(true);
      searchProducts(store.id, memberQuery, controller.signal)
        .then(setMemberResults)
        .catch((cause: unknown) => {
          if (!controller.signal.aborted) toast.error(errorMessage(cause));
        })
        .finally(() => {
          if (!controller.signal.aborted) setMemberSearching(false);
        });
    }, 250);
    return () => {
      window.clearTimeout(handle);
      controller.abort();
      setMemberSearching(false);
    };
  }, [membersCollection, mode, store, memberQuery]);

  const demoMembersFor = useCallback(
    (collectionId: string): Product[] => {
      const ids = demoMembers[collectionId] ?? [];
      const all = demoProducts().products;
      return ids
        .map((id) => all.find((product) => product.id === id))
        .filter((product): product is NonNullable<typeof product> => Boolean(product))
        .map((product) => demoProductRecord(product));
    },
    [demoMembers],
  );

  const openCreateCategory = (parent?: Category) => {
    setEditingCategory(null);
    setPendingParentId(parent?.id ?? "");
    setCategoryDialogOpen(true);
  };

  const openEditCategory = (category: Category) => {
    setEditingCategory(category);
    setPendingParentId(category.parent_id ?? "");
    setCategoryDialogOpen(true);
  };

  const saveCategory = async (draft: CategoryDraft) => {
    setCategoryBusy(true);
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingCategory) {
          setDemoCategories((current) =>
            current.map((category) =>
              category.id === editingCategory.id
                ? {
                    ...category,
                    title: draft.title,
                    handle: draft.handle,
                    parent_id: draft.parentId.length === 0 ? null : draft.parentId,
                    description: draft.description.length === 0 ? null : draft.description,
                    is_active: draft.isActive,
                    is_internal: draft.isInternal,
                    updated_at: now,
                  }
                : category,
            ),
          );
          toast.success("Category updated in the demo");
        } else {
          setDemoCategories((current) => [
            ...current,
            {
              id: `cat_demo_${Date.now()}`,
              parent_id: draft.parentId.length === 0 ? null : draft.parentId,
              title: draft.title,
              handle: draft.handle,
              description: draft.description.length === 0 ? null : draft.description,
              is_active: draft.isActive,
              is_internal: draft.isInternal,
              rank: current.length,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
          ]);
          toast.success("Category created in the demo");
        }
        if (draft.parentId.length > 0) {
          setExpanded((current) => new Set(current).add(draft.parentId));
        }
        setCategoryDialogOpen(false);
        return;
      }
      if (!store) throw new Error("No store is selected.");
      if (editingCategory) {
        const input: UpdateCategoryRequest = {
          title: draft.title,
          is_active: draft.isActive,
          is_internal: draft.isInternal,
          parent_id: draft.parentId,
        };
        if (draft.handle.length > 0) input.handle = draft.handle;
        // An empty description clears it; the API trims and stores null.
        input.description = draft.description;
        await updateCategory(store.id, editingCategory.id, input);
        toast.success("Category updated");
      } else {
        const input: CreateCategoryRequest = {
          title: draft.title,
          is_active: draft.isActive,
          is_internal: draft.isInternal,
        };
        if (draft.handle.length > 0) input.handle = draft.handle;
        if (draft.description.length > 0) input.description = draft.description;
        if (draft.parentId.length > 0) input.parent_id = draft.parentId;
        await createCategory(store.id, input);
        toast.success("Category created");
      }
      setCategoryDialogOpen(false);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCategoryBusy(false);
    }
  };

  const confirmDeleteCategory = async () => {
    if (!deleteCategoryTarget || categoryBusy) return;
    const target = deleteCategoryTarget;
    setCategoryBusy(true);
    try {
      if (mode === "demo") {
        setDemoCategories((current) =>
          current.filter(
            (category) => category.id !== target.id && category.parent_id !== target.id,
          ),
        );
      } else {
        if (!store) throw new Error("No store is selected.");
        await deleteCategory(store.id, target.id);
      }
      setDeleteCategoryTarget(null);
      toast.success("Category deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCategoryBusy(false);
    }
  };

  const saveCollection = async (draft: CollectionDraft) => {
    setCollectionBusy(true);
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingCollection) {
          setDemoCollections((current) =>
            current.map((collection) =>
              collection.id === editingCollection.id
                ? {
                    ...collection,
                    title: draft.title,
                    handle: draft.handle,
                    description: draft.description.length === 0 ? null : draft.description,
                    is_published: draft.isPublished,
                    updated_at: now,
                  }
                : collection,
            ),
          );
          toast.success("Collection updated in the demo");
        } else {
          const id = `col_demo_${Date.now()}`;
          setDemoCollections((current) => [
            ...current,
            {
              id,
              title: draft.title,
              handle: draft.handle,
              description: draft.description.length === 0 ? null : draft.description,
              is_published: draft.isPublished,
              rank: current.length,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
          ]);
          setDemoMembers((current) => ({ ...current, [id]: [] }));
          toast.success("Collection created in the demo");
        }
        setCollectionDialogOpen(false);
        return;
      }
      if (!store) throw new Error("No store is selected.");
      if (editingCollection) {
        const input: UpdateCollectionRequest = {
          title: draft.title,
          is_published: draft.isPublished,
        };
        if (draft.handle.length > 0) input.handle = draft.handle;
        input.description = draft.description;
        await updateCollection(store.id, editingCollection.id, input);
        toast.success("Collection updated");
      } else {
        const input: CreateCollectionRequest = {
          title: draft.title,
          is_published: draft.isPublished,
        };
        if (draft.handle.length > 0) input.handle = draft.handle;
        if (draft.description.length > 0) input.description = draft.description;
        await createCollection(store.id, input);
        toast.success("Collection created");
      }
      setCollectionDialogOpen(false);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCollectionBusy(false);
    }
  };

  const togglePublish = async (collection: Collection) => {
    if (collectionBusy) return;
    setCollectionBusy(true);
    try {
      if (mode === "demo") {
        setDemoCollections((current) =>
          current.map((entry) =>
            entry.id === collection.id ? { ...entry, is_published: !entry.is_published } : entry,
          ),
        );
      } else {
        if (!store) throw new Error("No store is selected.");
        if (collection.is_published) {
          await unpublishCollection(store.id, collection.id);
        } else {
          await publishCollection(store.id, collection.id);
        }
      }
      toast.success(collection.is_published ? "Collection unpublished" : "Collection published");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCollectionBusy(false);
    }
  };

  const confirmDeleteCollection = async () => {
    if (!deleteCollectionTarget || collectionBusy) return;
    const target = deleteCollectionTarget;
    setCollectionBusy(true);
    try {
      if (mode === "demo") {
        setDemoCollections((current) => current.filter((entry) => entry.id !== target.id));
      } else {
        if (!store) throw new Error("No store is selected.");
        await deleteCollection(store.id, target.id);
      }
      setDeleteCollectionTarget(null);
      if (membersCollection?.id === target.id) setMembersCollection(null);
      toast.success("Collection deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCollectionBusy(false);
    }
  };

  /** Search results that can still be added to the open collection. */
  const addableProducts = useMemo(() => {
    const needle = memberQuery.trim().toLowerCase();
    return memberResults
      .filter((product) => !memberProducts.some((entry) => entry.id === product.id))
      .filter(
        (product) =>
          needle.length === 0 ||
          product.title.toLowerCase().includes(needle) ||
          product.handle.toLowerCase().includes(needle),
      );
  }, [memberResults, memberProducts, memberQuery]);

  const openMembers = (collection: Collection) => {
    setMembersCollection(collection);
    setMemberQuery("");
    if (mode === "demo") {
      setMemberResults(demoProducts().products.map((product) => demoProductRecord(product)));
      setMemberProducts(demoMembersFor(collection.id));
    } else {
      setMemberResults([]);
      setMemberProducts([]);
    }
  };

  const addMember = async (product: Product) => {
    if (!membersCollection || memberBusyId) return;
    const collection = membersCollection;
    setMemberBusyId(product.id);
    try {
      if (mode === "demo") {
        const existing = demoMembers[collection.id] ?? [];
        if (!existing.includes(product.id)) {
          setDemoMembers((current) => ({
            ...current,
            [collection.id]: [...existing, product.id],
          }));
          setMemberProducts((current) =>
            current.some((entry) => entry.id === product.id) ? current : [...current, product],
          );
        }
      } else {
        if (!store) throw new Error("No store is selected.");
        await addProductsToCollection(store.id, collection.id, [product.id]);
        setMemberProducts((current) =>
          current.some((entry) => entry.id === product.id) ? current : [...current, product],
        );
      }
      toast.success(`${product.title} added to the collection`);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setMemberBusyId(null);
    }
  };

  const removeMember = async (product: Product) => {
    if (!membersCollection || memberBusyId) return;
    const collection = membersCollection;
    setMemberBusyId(product.id);
    try {
      if (mode === "demo") {
        setDemoMembers((current) => ({
          ...current,
          [collection.id]: (current[collection.id] ?? []).filter((id) => id !== product.id),
        }));
      } else {
        if (!store) throw new Error("No store is selected.");
        await removeProductFromCollection(store.id, collection.id, product.id);
      }
      setMemberProducts((current) => current.filter((entry) => entry.id !== product.id));
      toast.success(`${product.title} removed from the collection`);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setMemberBusyId(null);
    }
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const writable = can("products.write");
  const counts = collectionCounts(collections);
  const options = categoryParentOptions(categories, editingCategory?.id);

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Taxonomy"
        description="Organize the catalog with nested categories and curated collections."
        actions={
          writable ? (
            tab === "categories" ? (
              <Button onClick={() => openCreateCategory()}>
                <Plus className="size-4" aria-hidden="true" />
                New category
              </Button>
            ) : (
              <Button
                onClick={() => {
                  setEditingCollection(null);
                  setCollectionDialogOpen(true);
                }}
              >
                <Plus className="size-4" aria-hidden="true" />
                New collection
              </Button>
            )
          ) : null
        }
      />

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="categories">
            <FolderTree className="size-3.5" aria-hidden="true" />
            Categories
            <span className="font-mono text-[10px] text-faint tnum">{categories.length}</span>
          </TabsTrigger>
          <TabsTrigger value="collections">
            <Layers3 className="size-3.5" aria-hidden="true" />
            Collections
            <span className="font-mono text-[10px] text-faint tnum">{collections.length}</span>
          </TabsTrigger>
        </TabsList>

        <TabsContent value="categories" className="mt-4 space-y-4">
          <div className="relative w-full sm:w-[320px]">
            <Search
              className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
              aria-hidden="true"
            />
            <Input
              value={categorySearch}
              onChange={(event) => setCategorySearch(event.target.value)}
              placeholder="Search categories"
              className="pr-9 pl-9"
              aria-label="Search categories"
            />
            {categorySearch.length > 0 ? (
              <button
                type="button"
                onClick={() => setCategorySearch("")}
                className="absolute top-1/2 right-2.5 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-faint hover:bg-accent hover:text-foreground"
                aria-label="Clear search"
              >
                <X className="size-3.5" />
              </button>
            ) : null}
          </div>

          <Card className="overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-6">Category</TableHead>
                  <TableHead className="hidden w-[160px] md:table-cell">Handle</TableHead>
                  <TableHead className="w-[110px]">State</TableHead>
                  <TableHead className="hidden w-[130px] lg:table-cell">Updated</TableHead>
                  <TableHead className="w-10 pr-5" aria-label="Actions" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {visibleCategoryRows.map((node) => {
                  const hasChildren = node.children.length > 0;
                  const isExpanded = expanded.has(node.category.id);
                  return (
                    <TableRow key={node.category.id} className="h-[54px]">
                      <TableCell className="pl-6">
                        <div
                          className="flex items-center gap-2"
                          style={{ paddingLeft: node.depth * 20 }}
                        >
                          {hasChildren ? (
                            <button
                              type="button"
                              onClick={() =>
                                setExpanded((current) => {
                                  const next = new Set(current);
                                  if (next.has(node.category.id)) next.delete(node.category.id);
                                  else next.add(node.category.id);
                                  return next;
                                })
                              }
                              className="flex size-5 cursor-pointer items-center justify-center rounded text-faint hover:bg-accent hover:text-foreground"
                              aria-label={
                                isExpanded
                                  ? `Collapse ${node.category.title}`
                                  : `Expand ${node.category.title}`
                              }
                              aria-expanded={isExpanded}
                            >
                              {isExpanded ? (
                                <ChevronDown className="size-3.5" />
                              ) : (
                                <ChevronRight className="size-3.5" />
                              )}
                            </button>
                          ) : (
                            <span className="size-5" aria-hidden="true" />
                          )}
                          <div className="min-w-0">
                            <p className="truncate text-[12.5px] font-medium text-foreground">
                              {node.category.title}
                            </p>
                            <p className="mt-0.5 truncate font-mono text-[10px] text-faint">
                              {node.category.id}
                            </p>
                          </div>
                        </div>
                      </TableCell>
                      <TableCell className="hidden font-mono text-[11px] text-muted-foreground md:table-cell">
                        {node.category.handle}
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1.5">
                          <Badge variant={node.category.is_active ? "brand" : "secondary"}>
                            {node.category.is_active ? "Active" : "Inactive"}
                          </Badge>
                          {node.category.is_internal ? (
                            <Badge variant="outline">Internal</Badge>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell className="hidden text-[11.5px] text-muted-foreground lg:table-cell">
                        {shortDate(fromUnixSeconds(node.category.updated_at))}
                      </TableCell>
                      <TableCell className="pr-5 text-right">
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              aria-label={`Actions for ${node.category.title}`}
                              className="text-faint data-[state=open]:bg-accent"
                            >
                              <MoreHorizontal className="size-4" />
                            </Button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end" className="w-[210px]">
                            <DropdownMenuLabel className="truncate">
                              {node.category.title}
                            </DropdownMenuLabel>
                            <DropdownMenuItem
                              disabled={!writable}
                              onSelect={() => openEditCategory(node.category)}
                            >
                              <Pencil aria-hidden="true" />
                              Edit category
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              disabled={!writable}
                              onSelect={() => openCreateCategory(node.category)}
                            >
                              <Plus aria-hidden="true" />
                              Add subcategory
                            </DropdownMenuItem>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              variant="destructive"
                              disabled={!writable}
                              onSelect={() => setDeleteCategoryTarget(node.category)}
                            >
                              <Trash2 aria-hidden="true" />
                              Delete
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </TableRow>
                  );
                })}
                {visibleCategoryRows.length === 0 ? (
                  <TableRow className="hover:bg-transparent">
                    <TableCell colSpan={5} className="p-0">
                      <EmptyState
                        title={
                          categories.length === 0 ? "No categories yet" : "No matching categories"
                        }
                        description={
                          categories.length === 0
                            ? "Create a root category, then nest subcategories under it."
                            : "Try a different title or handle."
                        }
                        action={
                          categories.length === 0 && writable ? (
                            <Button onClick={() => openCreateCategory()}>
                              <Plus className="size-4" aria-hidden="true" />
                              New category
                            </Button>
                          ) : null
                        }
                      />
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </Card>
        </TabsContent>

        <TabsContent value="collections" className="mt-4 space-y-4">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
            <div className="relative w-full sm:w-[320px]">
              <Search
                className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
                aria-hidden="true"
              />
              <Input
                value={collectionSearch}
                onChange={(event) => setCollectionSearch(event.target.value)}
                placeholder="Search collections"
                className="pr-9 pl-9"
                aria-label="Search collections"
              />
              {collectionSearch.length > 0 ? (
                <button
                  type="button"
                  onClick={() => setCollectionSearch("")}
                  className="absolute top-1/2 right-2.5 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-faint hover:bg-accent hover:text-foreground"
                  aria-label="Clear search"
                >
                  <X className="size-3.5" />
                </button>
              ) : null}
            </div>
            <p className="text-[11.5px] text-muted-foreground">
              {counts.published} published · {counts.drafts} draft
              {mode === "demo" ? " · sample data" : ""}
            </p>
          </div>

          <Card className="overflow-hidden">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-6">Collection</TableHead>
                  <TableHead className="hidden w-[160px] md:table-cell">Handle</TableHead>
                  <TableHead className="w-[120px]">State</TableHead>
                  <TableHead className="hidden w-[130px] lg:table-cell">Updated</TableHead>
                  <TableHead className="w-10 pr-5" aria-label="Actions" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredCollections.map((collection) => (
                  <TableRow key={collection.id} className="h-[58px]">
                    <TableCell className="pl-6">
                      <p className="text-[12.5px] font-medium text-foreground">
                        {collection.title}
                      </p>
                      <p className="mt-0.5 max-w-[360px] truncate text-[10.5px] text-faint">
                        {collection.description ?? collection.id}
                      </p>
                    </TableCell>
                    <TableCell className="hidden font-mono text-[11px] text-muted-foreground md:table-cell">
                      {collection.handle}
                    </TableCell>
                    <TableCell>
                      <Badge variant={collection.is_published ? "brand" : "secondary"}>
                        {collection.is_published ? "Published" : "Draft"}
                      </Badge>
                    </TableCell>
                    <TableCell className="hidden text-[11.5px] text-muted-foreground lg:table-cell">
                      {shortDate(fromUnixSeconds(collection.updated_at))}
                    </TableCell>
                    <TableCell className="pr-5 text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={`Actions for ${collection.title}`}
                            className="text-faint data-[state=open]:bg-accent"
                          >
                            <MoreHorizontal className="size-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-[220px]">
                          <DropdownMenuLabel className="truncate">
                            {collection.title}
                          </DropdownMenuLabel>
                          <DropdownMenuItem
                            disabled={!writable}
                            onSelect={() => {
                              setEditingCollection(collection);
                              setCollectionDialogOpen(true);
                            }}
                          >
                            <Pencil aria-hidden="true" />
                            Edit collection
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            disabled={!writable || collectionBusy}
                            onSelect={() => void togglePublish(collection)}
                          >
                            <Layers3 aria-hidden="true" />
                            {collection.is_published ? "Unpublish" : "Publish"}
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            disabled={!writable}
                            onSelect={() => openMembers(collection)}
                          >
                            <Plus aria-hidden="true" />
                            Manage products
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            variant="destructive"
                            disabled={!writable}
                            onSelect={() => setDeleteCollectionTarget(collection)}
                          >
                            <Trash2 aria-hidden="true" />
                            Delete
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </TableCell>
                  </TableRow>
                ))}
                {filteredCollections.length === 0 ? (
                  <TableRow className="hover:bg-transparent">
                    <TableCell colSpan={5} className="p-0">
                      <EmptyState
                        title={
                          collections.length === 0
                            ? "No collections yet"
                            : "No matching collections"
                        }
                        description={
                          collections.length === 0
                            ? "Curate products into a collection, then publish it to the storefront."
                            : "Try a different title or handle."
                        }
                        action={
                          collections.length === 0 && writable ? (
                            <Button
                              onClick={() => {
                                setEditingCollection(null);
                                setCollectionDialogOpen(true);
                              }}
                            >
                              <Plus className="size-4" aria-hidden="true" />
                              New collection
                            </Button>
                          ) : null
                        }
                      />
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </Card>
        </TabsContent>
      </Tabs>

      <CategoryDialog
        open={categoryDialogOpen}
        onOpenChange={(open) => {
          setCategoryDialogOpen(open);
          if (!open) setEditingCategory(null);
        }}
        category={editingCategory}
        initialParentId={pendingParentId}
        parentOptions={options}
        busy={categoryBusy}
        onSubmit={saveCategory}
      />

      <CollectionDialog
        open={collectionDialogOpen}
        onOpenChange={(open) => {
          setCollectionDialogOpen(open);
          if (!open) setEditingCollection(null);
        }}
        collection={editingCollection}
        busy={collectionBusy}
        onSubmit={saveCollection}
      />

      <Dialog
        open={deleteCategoryTarget !== null}
        onOpenChange={(open) => {
          if (!open && !categoryBusy) setDeleteCategoryTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete category?</DialogTitle>
            <DialogDescription>
              “{deleteCategoryTarget?.title}” can only be deleted when no product references it. In
              the demo its direct children are removed too.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="ghost"
              onClick={() => setDeleteCategoryTarget(null)}
              disabled={categoryBusy}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void confirmDeleteCategory()}
              disabled={categoryBusy}
            >
              {categoryBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={deleteCollectionTarget !== null}
        onOpenChange={(open) => {
          if (!open && !collectionBusy) setDeleteCollectionTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete collection?</DialogTitle>
            <DialogDescription>
              “{deleteCollectionTarget?.title}” is removed. Products stay in the catalog.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="ghost"
              onClick={() => setDeleteCollectionTarget(null)}
              disabled={collectionBusy}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void confirmDeleteCollection()}
              disabled={collectionBusy}
            >
              {collectionBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={membersCollection !== null}
        onOpenChange={(open) => {
          if (!open) {
            setMembersCollection(null);
            setMemberQuery("");
            setMemberResults([]);
            setMemberProducts([]);
          }
        }}
      >
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Products in {membersCollection?.title}</DialogTitle>
            <DialogDescription>
              Add products from the catalog or remove the ones already linked.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            <div className="space-y-2">
              <Label>Current products</Label>
              {memberLoading ? (
                <p className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
                  <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                  Loading products…
                </p>
              ) : memberProducts.length === 0 ? (
                <p className="rounded-lg border border-dashed border-border px-3 py-3 text-[11.5px] text-muted-foreground">
                  No products in this collection yet.
                </p>
              ) : (
                <ul className="divide-y divide-border overflow-hidden rounded-lg border border-border">
                  {memberProducts.map((product) => (
                    <li
                      key={product.id}
                      className="flex items-center gap-3 bg-background px-3 py-2"
                    >
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-[12px] text-foreground">{product.title}</p>
                        <p className="truncate font-mono text-[10px] text-faint">
                          {product.handle}
                        </p>
                      </div>
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={memberBusyId !== null || !writable}
                        onClick={() => void removeMember(product)}
                      >
                        {memberBusyId === product.id ? (
                          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                        ) : (
                          <X className="size-3.5" aria-hidden="true" />
                        )}
                        Remove
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </div>

            <div className="space-y-2 border-t border-border pt-4">
              <Label htmlFor="collection-member-search">Add a product</Label>
              <div className="relative">
                <Search
                  className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
                  aria-hidden="true"
                />
                <Input
                  id="collection-member-search"
                  value={memberQuery}
                  onChange={(event) => setMemberQuery(event.target.value)}
                  placeholder="Search products by title or handle"
                  className="pl-9"
                />
              </div>
              {memberSearching ? (
                <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                  Searching products…
                </p>
              ) : null}
              <ul className="divide-y divide-border overflow-hidden rounded-lg border border-border">
                {addableProducts.slice(0, 6).map((product) => (
                  <li key={product.id} className="flex items-center gap-3 bg-background px-3 py-2">
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-[12px] text-foreground">{product.title}</p>
                      <p className="truncate font-mono text-[10px] text-faint">{product.handle}</p>
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={memberBusyId !== null || !writable}
                      onClick={() => void addMember(product)}
                    >
                      {memberBusyId === product.id ? (
                        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                      ) : (
                        <Plus className="size-3.5" aria-hidden="true" />
                      )}
                      Add
                    </Button>
                  </li>
                ))}
                {addableProducts.length === 0 ? (
                  <li className="bg-background px-3 py-3 text-[11.5px] text-muted-foreground">
                    {memberQuery.trim().length === 0
                      ? "All matching products are already in this collection."
                      : "No products match that search."}
                  </li>
                ) : null}
              </ul>
            </div>
          </div>

          <DialogFooter>
            <Button onClick={() => setMembersCollection(null)}>Done</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** Ids of every ancestor of a category, walking parent links in the input. */
function ancestorIds(categories: readonly Category[], id: string): string[] {
  const byId = new Map(categories.map((category) => [category.id, category]));
  const ancestors: string[] = [];
  const seen = new Set<string>([id]);
  let current = byId.get(id)?.parent_id ?? null;
  while (current !== null && !seen.has(current)) {
    seen.add(current);
    ancestors.push(current);
    current = byId.get(current)?.parent_id ?? null;
  }
  return ancestors;
}

/** Shapes a demo product view into the fields the member picker reads. */
function demoProductRecord(view: {
  id: string;
  title: string;
  handle: string;
  status: Product["status"];
}): Product {
  return {
    id: view.id,
    title: view.title,
    subtitle: null,
    description: null,
    handle: view.handle,
    status: view.status,
    thumbnail: null,
    weight: null,
    dimensions: { length: null, height: null, width: null },
    metadata: {},
    created_at: 0,
    updated_at: 0,
    variants: [],
    options: [],
    images: [],
    categories: [],
    collections: [],
    tags: [],
  };
}

function CategoryDialog({
  open,
  onOpenChange,
  category,
  initialParentId,
  parentOptions,
  busy,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  category: Category | null;
  initialParentId: string;
  parentOptions: Array<{ id: string; title: string; depth: number }>;
  busy: boolean;
  onSubmit: (draft: CategoryDraft) => Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [handle, setHandle] = useState("");
  const [parentId, setParentId] = useState("");
  const [description, setDescription] = useState("");
  const [isActive, setIsActive] = useState(true);
  const [isInternal, setIsInternal] = useState(false);
  const [titleError, setTitleError] = useState<string | null>(null);
  const [handleFieldError, setHandleFieldError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setTitle(category?.title ?? "");
    setHandle(category?.handle ?? "");
    setParentId(initialParentId);
    setDescription(category?.description ?? "");
    setIsActive(category?.is_active ?? true);
    setIsInternal(category?.is_internal ?? false);
    setTitleError(null);
    setHandleFieldError(null);
  }, [open, category?.id, initialParentId]);

  const submit = async () => {
    if (busy) return;
    const trimmedTitle = title.trim();
    if (trimmedTitle.length === 0) {
      setTitleError("Enter a title for this category.");
      return;
    }
    const handleMessage = handleError(handle);
    if (handleMessage !== null) {
      setHandleFieldError(handleMessage);
      return;
    }
    await onSubmit({
      title: trimmedTitle,
      handle: handle.trim(),
      parentId,
      description: description.trim(),
      isActive,
      isInternal,
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{category ? "Edit category" : "New category"}</DialogTitle>
            <DialogDescription>
              Categories nest to any depth; the storefront navigation follows the tree order.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="category-title">Title</Label>
            <Input
              id="category-title"
              value={title}
              onChange={(event) => {
                setTitle(event.target.value);
                setTitleError(null);
              }}
              placeholder="Tableware"
              autoComplete="off"
              autoFocus
              disabled={busy}
              aria-invalid={titleError !== null}
            />
            {titleError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {titleError}
              </p>
            ) : null}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="category-handle">Handle</Label>
              <Input
                id="category-handle"
                value={handle}
                onChange={(event) => {
                  setHandle(event.target.value);
                  setHandleFieldError(null);
                }}
                placeholder="tableware"
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                aria-invalid={handleFieldError !== null}
                className="font-mono"
              />
              <p className="text-[11px] text-faint">Leave empty to derive it from the title.</p>
              {handleFieldError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {handleFieldError}
                </p>
              ) : null}
            </div>
            <div className="space-y-2">
              <Label htmlFor="category-parent">Parent category</Label>
              <div className="relative">
                <select
                  id="category-parent"
                  value={parentId}
                  onChange={(event) => setParentId(event.target.value)}
                  disabled={busy}
                  className={selectClass}
                >
                  <option value="">No parent (root)</option>
                  {parentOptions.map((option) => (
                    <option key={option.id} value={option.id}>
                      {"— ".repeat(option.depth)}
                      {option.title}
                    </option>
                  ))}
                </select>
                <ChevronDown
                  className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                  aria-hidden="true"
                />
              </div>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="category-description">Description</Label>
            <textarea
              id="category-description"
              rows={3}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              disabled={busy}
              placeholder="Plates, bowls and carafes."
              className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs transition-[color,box-shadow,border-color] outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
            />
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3">
              <div>
                <Label htmlFor="category-active">Active</Label>
                <p className="mt-1 text-[11px] text-muted-foreground">Visible on the storefront.</p>
              </div>
              <Switch
                id="category-active"
                checked={isActive}
                onCheckedChange={setIsActive}
                disabled={busy}
              />
            </div>
            <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3">
              <div>
                <Label htmlFor="category-internal">Internal only</Label>
                <p className="mt-1 text-[11px] text-muted-foreground">Hide from the storefront.</p>
              </div>
              <Switch
                id="category-internal"
                checked={isInternal}
                onCheckedChange={setIsInternal}
                disabled={busy}
              />
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
              disabled={busy}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {category ? "Save changes" : "Create category"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function CollectionDialog({
  open,
  onOpenChange,
  collection,
  busy,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  collection: Collection | null;
  busy: boolean;
  onSubmit: (draft: CollectionDraft) => Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [handle, setHandle] = useState("");
  const [description, setDescription] = useState("");
  const [isPublished, setIsPublished] = useState(false);
  const [titleError, setTitleError] = useState<string | null>(null);
  const [handleFieldError, setHandleFieldError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setTitle(collection?.title ?? "");
    setHandle(collection?.handle ?? "");
    setDescription(collection?.description ?? "");
    setIsPublished(collection?.is_published ?? false);
    setTitleError(null);
    setHandleFieldError(null);
  }, [open, collection?.id]);

  const submit = async () => {
    if (busy) return;
    const trimmedTitle = title.trim();
    if (trimmedTitle.length === 0) {
      setTitleError("Enter a title for this collection.");
      return;
    }
    const handleMessage = handleError(handle);
    if (handleMessage !== null) {
      setHandleFieldError(handleMessage);
      return;
    }
    await onSubmit({
      title: trimmedTitle,
      handle: handle.trim(),
      description: description.trim(),
      isPublished,
    });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{collection ? "Edit collection" : "New collection"}</DialogTitle>
            <DialogDescription>
              Collections are curated product lists. Publish one when it is ready for the
              storefront.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="collection-title">Title</Label>
              <Input
                id="collection-title"
                value={title}
                onChange={(event) => {
                  setTitle(event.target.value);
                  setTitleError(null);
                }}
                placeholder="New arrivals"
                autoComplete="off"
                autoFocus
                disabled={busy}
                aria-invalid={titleError !== null}
              />
              {titleError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {titleError}
                </p>
              ) : null}
            </div>
            <div className="space-y-2">
              <Label htmlFor="collection-handle">Handle</Label>
              <Input
                id="collection-handle"
                value={handle}
                onChange={(event) => {
                  setHandle(event.target.value);
                  setHandleFieldError(null);
                }}
                placeholder="new-arrivals"
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                aria-invalid={handleFieldError !== null}
                className="font-mono"
              />
              {handleFieldError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {handleFieldError}
                </p>
              ) : null}
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="collection-description">Description</Label>
            <textarea
              id="collection-description"
              rows={3}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              disabled={busy}
              placeholder="The latest additions to the catalog."
              className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs transition-[color,box-shadow,border-color] outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
            />
          </div>

          <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3.5">
            <div>
              <Label htmlFor="collection-published">Published</Label>
              <p className="mt-1 text-[11.5px] text-muted-foreground">
                Published collections are served by the storefront.
              </p>
            </div>
            <Switch
              id="collection-published"
              checked={isPublished}
              onCheckedChange={setIsPublished}
              disabled={busy}
            />
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
              disabled={busy}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {collection ? "Save changes" : "Create collection"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
