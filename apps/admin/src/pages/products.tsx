import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowUpRight,
  Check,
  ChevronDown,
  Download,
  LayoutGrid,
  List,
  Loader2,
  Plus,
  Search,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/page-header";
import { ProductThumb } from "@/components/product-thumb";
import { ProgressBar } from "@/components/progress-bar";
import { ProductStatusPill } from "@/components/status-pill";
import { ErrorState, PageSkeleton } from "@/components/states";
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
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAuth } from "@/lib/auth";
import { errorMessage } from "@/lib/api";
import { consumePendingCommand, onCommand } from "@/lib/commands";
import { demoProducts } from "@/lib/demo";
import { downloadCsv, moneyShort, shortDate } from "@/lib/format";
import { createProduct, loadProducts } from "@/lib/loaders";
import { swatchFor, type ProductStatusView, type ProductView } from "@/lib/models";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

type ViewMode = "grid" | "list";
type SortMode = "newest" | "name" | "price";

const STATUS_TABS: Array<{ value: "all" | ProductStatusView; label: string }> = [
  { value: "all", label: "All" },
  { value: "active", label: "Active" },
  { value: "draft", label: "Draft" },
  { value: "archived", label: "Archived" },
];

const SORT_OPTIONS: Array<{ value: SortMode; label: string }> = [
  { value: "newest", label: "Newest first" },
  { value: "name", label: "Name A–Z" },
  { value: "price", label: "Highest price" },
];

function stockTone(available: number): "brand" | "warning" | "destructive" {
  if (available <= 0) return "destructive";
  if (available <= 10) return "warning";
  return "brand";
}

export function ProductsPage() {
  const { mode, client, store, dataKey } = useAuth();
  const [status, setStatus] = useState<"all" | ProductStatusView>("all");
  const [query, setQuery] = useState("");
  const [collection, setCollection] = useState<string | null>(null);
  const [view, setView] = useState<ViewMode>("grid");
  const [sort, setSort] = useState<SortMode>("newest");
  const [extra, setExtra] = useState<ProductView[]>([]);
  const [addOpen, setAddOpen] = useState(false);

  useEffect(() => {
    setExtra([]);
    setCollection(null);
    setStatus("all");
    setQuery("");
  }, [store?.id, dataKey]);

  useEffect(() => {
    const pending = consumePendingCommand();
    if (pending === "new-product") setAddOpen(true);
    return onCommand((command) => {
      if (command === "new-product") setAddOpen(true);
    });
  }, []);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") return Promise.resolve(demoProducts());
      if (!client || !store) return Promise.reject(new Error("No store is selected."));
      return loadProducts(client, store, signal);
    },
    [mode, client, store],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);

  const products = useMemo(() => [...extra, ...(data?.products ?? [])], [extra, data?.products]);
  const collections = data?.collections ?? [];

  const counts = useMemo(
    () => ({
      all: products.length,
      active: products.filter((product) => product.status === "active").length,
      draft: products.filter((product) => product.status === "draft").length,
      archived: products.filter((product) => product.status === "archived").length,
    }),
    [products],
  );

  const collectionCount = new Set(products.map((product) => product.collection)).size;
  const currency = store?.currency_code ?? "USD";
  const maxStock = Math.max(1, ...products.map((product) => product.stock));

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const list = products.filter((product) => {
      if (status !== "all" && product.status !== status) return false;
      if (collection !== null && product.collection !== collection) return false;
      if (needle.length === 0) return true;
      return (
        product.title.toLowerCase().includes(needle) ||
        product.handle.toLowerCase().includes(needle) ||
        product.collection.toLowerCase().includes(needle)
      );
    });
    return list.sort((a, b) => {
      if (sort === "name") return a.title.localeCompare(b.title);
      if (sort === "price") return (b.price ?? 0) - (a.price ?? 0);
      return b.updatedAt.localeCompare(a.updatedAt);
    });
  }, [products, status, collection, query, sort]);

  const exportProducts = () => {
    downloadCsv(
      "fastcart-products.csv",
      ["Title", "Handle", "Collection", "Status", "Price", "Stock", "Updated"],
      filtered.map((product) => [
        product.title,
        product.handle,
        product.collection,
        product.status,
        product.price?.toFixed(2) ?? "",
        product.stock,
        product.updatedAt,
      ]),
    );
  };

  if (!data && loading) return <PageSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Products"
        description={`${products.length} ${products.length === 1 ? "product" : "products"} across ${collectionCount} ${collectionCount === 1 ? "collection" : "collections"}`}
        actions={
          <>
            <Button variant="outline" onClick={exportProducts} disabled={filtered.length === 0}>
              <Download className="size-4" aria-hidden="true" />
              Export
            </Button>
            <Button onClick={() => setAddOpen(true)}>
              <Plus className="size-4" aria-hidden="true" />
              Add product
            </Button>
          </>
        }
      />

      <div className="flex flex-col gap-3 xl:flex-row xl:items-center">
        <div className="relative w-full xl:w-[280px]">
          <Search
            className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
            aria-hidden="true"
          />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search products"
            className="pr-9 pl-9"
            aria-label="Search products"
          />
          {query.length > 0 ? (
            <button
              type="button"
              onClick={() => setQuery("")}
              className="absolute top-1/2 right-2.5 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-faint hover:bg-accent hover:text-foreground"
              aria-label="Clear search"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>

        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto rounded-full border border-border bg-secondary p-1 no-scrollbar">
          {STATUS_TABS.map((tab) => (
            <button
              key={tab.value}
              type="button"
              onClick={() => setStatus(tab.value)}
              aria-pressed={status === tab.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center gap-1.5 rounded-full px-3 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                status === tab.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {tab.label}
              <span className="font-mono text-[10px] text-faint tnum">{counts[tab.value]}</span>
            </button>
          ))}
          {collection ? (
            <button
              type="button"
              onClick={() => setCollection(null)}
              className="ml-1 inline-flex h-7 shrink-0 cursor-pointer items-center gap-1 rounded-full bg-card px-2.5 text-[11px] text-ink-soft shadow-xs"
            >
              {collection}
              <X className="size-3" aria-hidden="true" />
            </button>
          ) : null}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <div className="flex h-10 items-center rounded-md border border-border bg-card p-0.5 shadow-xs">
            <button
              type="button"
              onClick={() => setView("grid")}
              aria-pressed={view === "grid"}
              aria-label="Grid view"
              className={cn(
                "flex size-8 cursor-pointer items-center justify-center rounded-[7px] transition-colors",
                view === "grid"
                  ? "bg-secondary text-foreground"
                  : "text-faint hover:text-foreground",
              )}
            >
              <LayoutGrid className="size-4" />
            </button>
            <button
              type="button"
              onClick={() => setView("list")}
              aria-pressed={view === "list"}
              aria-label="List view"
              className={cn(
                "flex size-8 cursor-pointer items-center justify-center rounded-[7px] transition-colors",
                view === "list"
                  ? "bg-secondary text-foreground"
                  : "text-faint hover:text-foreground",
              )}
            >
              <List className="size-4" />
            </button>
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" className="justify-between gap-2">
                {SORT_OPTIONS.find((option) => option.value === sort)?.label}
                <ChevronDown className="size-3.5 text-faint" aria-hidden="true" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[170px]">
              <DropdownMenuLabel>Sort products</DropdownMenuLabel>
              {SORT_OPTIONS.map((option) => (
                <DropdownMenuItem key={option.value} onSelect={() => setSort(option.value)}>
                  {option.label}
                  {sort === option.value ? (
                    <Check className="ml-auto size-3.5 text-brand" aria-hidden="true" />
                  ) : null}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {filtered.length === 0 ? (
        <Card className="px-6 py-16 text-center">
          <p className="text-[13.5px] font-medium text-foreground">No products found</p>
          <p className="mt-1 text-[12.5px] text-muted-foreground">
            {query.length > 0 || status !== "all" || collection
              ? "Adjust the filters or search for a different product."
              : "Add your first product to start selling."}
          </p>
        </Card>
      ) : view === "grid" ? (
        <section
          aria-label="Product catalog"
          className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4"
        >
          {filtered.map((product) => (
            <Card
              key={product.id}
              className="overflow-hidden p-0 transition-colors duration-150 hover:border-line-strong"
            >
              <ProductThumb
                title={product.title}
                swatch={product.swatch}
                className="aspect-[4/3] w-full"
                initialClassName="text-[54px]"
              />
              <div className="p-4">
                <div className="flex items-center justify-between gap-2">
                  <ProductStatusPill status={product.status} />
                  <span className="truncate text-[10.5px] text-muted-foreground">
                    {product.collection}
                  </span>
                </div>
                <p className="mt-2.5 truncate text-[13px] font-medium text-foreground">
                  {product.title}
                </p>
                <p className="mt-1 text-[12px] text-muted-foreground">
                  {product.price === null ? "No price set" : moneyShort(product.price, currency)}
                </p>
                <div className="mt-4">
                  <div className="flex items-baseline justify-between gap-2">
                    <span
                      className={cn(
                        "text-[11px]",
                        product.stock <= 0
                          ? "text-destructive"
                          : product.stock <= 10
                            ? "text-warning"
                            : "text-muted-foreground",
                      )}
                    >
                      {product.stock <= 0 ? "Out of stock" : `${product.stock} in stock`}
                    </span>
                    <span className="font-mono text-[10.5px] text-faint tnum">{product.stock}</span>
                  </div>
                  <ProgressBar
                    className="mt-2 h-1"
                    value={product.stock}
                    max={maxStock}
                    tone={stockTone(product.stock)}
                  />
                </div>
              </div>
            </Card>
          ))}
        </section>
      ) : (
        <Card className="overflow-hidden">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-6">Product</TableHead>
                <TableHead>Collection</TableHead>
                <TableHead className="w-[120px]">Status</TableHead>
                <TableHead className="w-[110px] text-right">Price</TableHead>
                <TableHead className="w-[110px] text-right">Stock</TableHead>
                <TableHead className="w-[120px] pr-6 text-right">Updated</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.map((product) => (
                <TableRow key={product.id} className="h-[58px]">
                  <TableCell className="pl-6">
                    <div className="flex items-center gap-3">
                      <ProductThumb
                        title={product.title}
                        swatch={product.swatch}
                        className="size-8 shrink-0 rounded-md"
                        initialClassName="text-[13px]"
                      />
                      <div className="min-w-0">
                        <p className="truncate text-[12.5px] font-medium text-foreground">
                          {product.title}
                        </p>
                        <p className="mt-0.5 truncate font-mono text-[10px] text-faint">
                          {product.handle}
                        </p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell className="text-[12px] text-muted-foreground">
                    {product.collection}
                  </TableCell>
                  <TableCell>
                    <ProductStatusPill status={product.status} />
                  </TableCell>
                  <TableCell className="text-right text-[12px] text-ink-soft tnum">
                    {product.price === null ? "—" : moneyShort(product.price, currency)}
                  </TableCell>
                  <TableCell
                    className={cn(
                      "text-right font-mono text-[11px] tnum",
                      product.stock <= 0
                        ? "text-destructive"
                        : product.stock <= 10
                          ? "text-warning"
                          : "text-ink-soft",
                    )}
                  >
                    {product.stock}
                  </TableCell>
                  <TableCell className="pr-6 text-right text-[11.5px] text-muted-foreground">
                    {shortDate(product.updatedAt)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>
      )}

      {collections.length > 0 ? (
        <section className="pt-2">
          <div className="mb-3 flex items-center justify-between">
            <h2 className="font-display text-[22px] leading-none text-foreground">Collections</h2>
            {collection ? (
              <button
                type="button"
                onClick={() => setCollection(null)}
                className="inline-flex cursor-pointer items-center gap-1 text-[11.5px] text-muted-foreground hover:text-foreground"
              >
                View all
                <ArrowUpRight className="size-3" aria-hidden="true" />
              </button>
            ) : null}
          </div>
          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
            {collections.map((entry) => (
              <button
                key={entry.title}
                type="button"
                onClick={() =>
                  setCollection((current) => (current === entry.title ? null : entry.title))
                }
                aria-pressed={collection === entry.title}
                className={cn(
                  "cursor-pointer rounded-xl border bg-card p-5 text-left shadow-soft transition-colors duration-150 hover:border-line-strong focus-visible:ring-[3px] focus-visible:ring-ring/30 focus-visible:outline-none",
                  collection === entry.title ? "border-brand/40" : "border-border",
                )}
              >
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <p className="text-[13px] font-medium text-foreground">{entry.title}</p>
                    <p className="mt-1 text-[11px] text-muted-foreground">
                      {entry.count} {entry.count === 1 ? "product" : "products"}
                    </p>
                  </div>
                  <ArrowUpRight className="size-3.5 text-faint" aria-hidden="true" />
                </div>
                <div className="mt-6 flex items-center gap-1.5">
                  {entry.colors.slice(0, 3).map((color, index) => (
                    <span
                      key={`${color}-${index}`}
                      className="size-3 rounded-full ring-1 ring-black/[0.06] dark:ring-white/10"
                      style={{ backgroundColor: color }}
                      aria-hidden="true"
                    />
                  ))}
                </div>
              </button>
            ))}
          </div>
        </section>
      ) : null}

      <AddProductDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        onCreated={(product) => {
          if (mode === "demo") setExtra((current) => [product, ...current]);
          else reload();
        }}
      />
    </div>
  );
}

function AddProductDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (product: ProductView) => void;
}) {
  const { mode, client } = useAuth();
  const [title, setTitle] = useState("");
  const [sku, setSku] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);

  const reset = () => {
    setTitle("");
    setSku("");
    setDescription("");
  };

  const submit = async () => {
    if (busy) return;
    if (title.trim().length === 0) {
      toast.error("Give the product a title.");
      return;
    }
    setBusy(true);
    try {
      if (mode === "demo") {
        const product: ProductView = {
          id: `demo-${Date.now()}`,
          title: title.trim(),
          handle: title
            .trim()
            .toLowerCase()
            .replace(/[^a-z0-9]+/g, "-")
            .replace(/(^-|-$)/g, ""),
          collection: "Uncategorized",
          price: null,
          stock: 0,
          status: "draft",
          updatedAt: new Date().toISOString(),
          swatch: swatchFor(title),
        };
        onCreated(product);
        toast.success("Draft product added to the demo catalog");
      } else {
        if (!client) throw new Error("No store is selected.");
        await createProduct(client, { title, sku, description });
        onCreated({
          id: "created",
          title: title.trim(),
          handle: "",
          collection: "Uncategorized",
          price: null,
          stock: 0,
          status: "draft",
          updatedAt: new Date().toISOString(),
          swatch: swatchFor(title),
        });
        toast.success("Draft product created. Add prices in a price list.");
      }
      reset();
      onOpenChange(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset();
        onOpenChange(next);
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Add product</DialogTitle>
          <DialogDescription>
            New products start as drafts. Prices live in price lists and can be added once the
            product exists.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="product-title">Title</Label>
            <Input
              id="product-title"
              placeholder="Ridge Ceramic Carafe"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              disabled={busy}
              autoFocus
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="product-sku">SKU</Label>
            <Input
              id="product-sku"
              placeholder="RCC-SAG-01"
              value={sku}
              onChange={(event) => setSku(event.target.value)}
              disabled={busy}
              className="font-mono"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="product-description">Description</Label>
            <textarea
              id="product-description"
              rows={3}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              disabled={busy}
              placeholder="Stoneware carafe with a matte sage glaze."
              className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs transition-[color,box-shadow,border-color] outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={busy}>
            {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
            Create draft
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
