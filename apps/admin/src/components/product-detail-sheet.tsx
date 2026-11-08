import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import {
  Archive,
  Check,
  ChevronDown,
  ChevronUp,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import type { CreateVariantRequest, Price, PriceList, Variant } from "@fastcart/shared-types";
import { toast } from "sonner";

import { ProductStatusPill, StockPill } from "@/components/status-pill";
import { ErrorState } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { moneyShort, shortDate } from "@/lib/format";
import { loadProductDetail, loadVariantPrices } from "@/lib/loaders";
import type { ProductDetailView, ProductView } from "@/lib/models";
import {
  formatMinorAmount,
  formatMinorInput,
  isValidCurrency,
  normalizeCurrency,
  parsePriceAmount,
} from "@/lib/pricing";
import { useAsyncData } from "@/lib/use-async-data";

interface ProductDetailSheetProps {
  product: ProductView;
  mode: "api" | "demo";
  storeCurrency: string;
  onClose: () => void;
  onMutated: () => void;
}

/**
 * Product authoring drawer used from the catalog. The live surface loads the
 * full product plus prices and availability when it opens; the demo surface
 * stays read-only because there is no API behind it.
 */
export function ProductDetailSheet({
  product,
  mode,
  storeCurrency,
  onClose,
  onMutated,
}: ProductDetailSheetProps) {
  if (mode === "demo") {
    return <DemoProductSheet product={product} storeCurrency={storeCurrency} onClose={onClose} />;
  }
  return (
    <LiveProductSheet
      product={product}
      storeCurrency={storeCurrency}
      onClose={onClose}
      onMutated={onMutated}
    />
  );
}

function DemoProductSheet({
  product,
  storeCurrency,
  onClose,
}: {
  product: ProductView;
  storeCurrency: string;
  onClose: () => void;
}) {
  return (
    <Sheet open onOpenChange={(next) => !next && onClose()}>
      <SheetContent side="right" className="w-full p-0 sm:max-w-xl">
        <SheetHeader className="pr-14">
          <div className="flex flex-wrap items-center gap-2">
            <SheetTitle className="min-w-0 truncate">{product.title}</SheetTitle>
            <ProductStatusPill status={product.status} />
          </div>
          <SheetDescription className="font-mono">{product.handle}</SheetDescription>
        </SheetHeader>
        <div className="flex-1 space-y-5 overflow-y-auto p-6">
          <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-[12px] leading-relaxed text-muted-foreground">
            Variants, publish controls and prices are available when the admin is connected to the
            Fastcart API. This demo catalog is read-only.
          </p>
          <dl className="grid grid-cols-2 gap-4 text-[12px]">
            <div>
              <dt className="text-[10.5px] font-medium text-faint">Collection</dt>
              <dd className="mt-0.5 text-foreground">{product.collection}</dd>
            </div>
            <div>
              <dt className="text-[10.5px] font-medium text-faint">Price</dt>
              <dd className="mt-0.5 text-foreground">
                {product.price === null ? "No price set" : moneyShort(product.price, storeCurrency)}
              </dd>
            </div>
            <div>
              <dt className="text-[10.5px] font-medium text-faint">Stock</dt>
              <dd className="mt-0.5 text-foreground">{product.stock} units</dd>
            </div>
            <div>
              <dt className="text-[10.5px] font-medium text-faint">Updated</dt>
              <dd className="mt-0.5 text-foreground">{shortDate(product.updatedAt)}</dd>
            </div>
          </dl>
        </div>
      </SheetContent>
    </Sheet>
  );
}

function LiveProductSheet({
  product,
  storeCurrency,
  onClose,
  onMutated,
}: {
  product: ProductView;
  storeCurrency: string;
  onClose: () => void;
  onMutated: () => void;
}) {
  const { client } = useAuth();
  const currency = normalizeCurrency(storeCurrency);
  const [busy, setBusy] = useState<"publish" | "archive" | null>(null);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadProductDetail(client, product.id, currency, signal);
    },
    [client, product.id, currency],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [product.id, currency]);

  const refresh = useCallback(() => {
    reload();
    onMutated();
  }, [reload, onMutated]);

  const changeStatus = async (action: "publish" | "archive") => {
    if (!client || !data || busy !== null) return;
    setBusy(action);
    try {
      if (action === "publish") await client.products.publish(product.id);
      else await client.products.archive(product.id);
      toast.success(action === "publish" ? "Product published." : "Product archived.");
      refresh();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const currentStatus = data?.product.status ?? product.status;
  const primarySku = data?.product.variants[0]?.sku ?? null;
  const actionsDisabled = data === null || busy !== null;

  return (
    <Sheet open onOpenChange={(next) => !next && onClose()}>
      <SheetContent side="right" className="flex w-full flex-col gap-0 p-0 sm:max-w-2xl">
        <SheetHeader className="pr-14">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <SheetTitle className="min-w-0 truncate">
                {data?.product.title ?? product.title}
              </SheetTitle>
              <ProductStatusPill status={currentStatus} />
            </div>
            <SheetDescription className="mt-1 truncate">
              {data?.product.handle ?? product.handle}
              {primarySku ? ` · ${primarySku}` : ""}
            </SheetDescription>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              onClick={() => void changeStatus("publish")}
              disabled={actionsDisabled || currentStatus === "active"}
            >
              {busy === "publish" ? (
                <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <Upload className="size-3.5" aria-hidden="true" />
              )}
              Publish
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => void changeStatus("archive")}
              disabled={actionsDisabled || currentStatus === "archived"}
            >
              {busy === "archive" ? (
                <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <Archive className="size-3.5" aria-hidden="true" />
              )}
              Archive
            </Button>
            <Button size="sm" variant="ghost" onClick={reload} disabled={loading}>
              <RefreshCw
                className={loading ? "size-3.5 animate-spin" : "size-3.5"}
                aria-hidden="true"
              />
              Refresh
            </Button>
          </div>
        </SheetHeader>

        <div className="flex-1 overflow-y-auto p-6">
          {!data && loading ? (
            <div
              className="flex items-center justify-center py-16"
              aria-busy="true"
              aria-label="Loading product"
            >
              <Loader2 className="size-5 animate-spin text-faint" aria-hidden="true" />
            </div>
          ) : !data && error ? (
            <ErrorState message={error} onRetry={reload} />
          ) : data ? (
            <div className="space-y-6">
              {error ? (
                <div
                  role="alert"
                  className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-[11.5px] text-destructive"
                >
                  <span>{error}</span>
                  <Button size="sm" variant="outline" onClick={reload}>
                    Try again
                  </Button>
                </div>
              ) : null}
              <VariantsPanel detail={data} defaultCurrency={currency} onChanged={refresh} />
            </div>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}

function VariantsPanel({
  detail,
  defaultCurrency,
  onChanged,
}: {
  detail: ProductDetailView;
  defaultCurrency: string;
  onChanged: () => void;
}) {
  const { client } = useAuth();
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [draftSku, setDraftSku] = useState("");
  const [draftManageInventory, setDraftManageInventory] = useState(true);
  const [savingVariant, setSavingVariant] = useState(false);

  const [pendingDelete, setPendingDelete] = useState<Variant | null>(null);
  const [deletingVariant, setDeletingVariant] = useState(false);

  const [expandedVariantId, setExpandedVariantId] = useState<string | null>(
    detail.product.variants[0]?.id ?? null,
  );

  const [newTitle, setNewTitle] = useState("");
  const [newSku, setNewSku] = useState("");
  const [newManageInventory, setNewManageInventory] = useState(true);
  const [addingVariant, setAddingVariant] = useState(false);

  const startEdit = (variant: Variant) => {
    setEditingId(variant.id);
    setDraftTitle(variant.title);
    setDraftSku(variant.sku ?? "");
    setDraftManageInventory(variant.manage_inventory);
  };

  const cancelEdit = () => setEditingId(null);

  const saveVariant = async (variant: Variant) => {
    if (!client || savingVariant) return;
    const title = draftTitle.trim();
    if (title.length === 0) {
      toast.error("Give the variant a title.");
      return;
    }
    setSavingVariant(true);
    try {
      await client.products.updateVariant(variant.id, {
        title,
        sku: draftSku.trim(),
        manage_inventory: draftManageInventory,
      });
      toast.success("Variant updated.");
      setEditingId(null);
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setSavingVariant(false);
    }
  };

  const addVariant = async () => {
    if (!client || addingVariant) return;
    const title = newTitle.trim();
    if (title.length === 0) {
      toast.error("Give the variant a title.");
      return;
    }
    const body: CreateVariantRequest = {
      title,
      manage_inventory: newManageInventory,
    };
    const sku = newSku.trim();
    if (sku.length > 0) body.sku = sku;

    setAddingVariant(true);
    try {
      const created = await client.products.createVariant(detail.product.id, body);
      setNewTitle("");
      setNewSku("");
      setNewManageInventory(true);
      setExpandedVariantId(created.id);
      toast.success("Variant added.");
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setAddingVariant(false);
    }
  };

  const removeVariant = async () => {
    if (!client || pendingDelete === null || deletingVariant) return;
    setDeletingVariant(true);
    try {
      await client.products.deleteVariant(pendingDelete.id);
      if (expandedVariantId === pendingDelete.id) setExpandedVariantId(null);
      if (editingId === pendingDelete.id) setEditingId(null);
      toast.success("Variant deleted.");
      setPendingDelete(null);
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDeletingVariant(false);
    }
  };

  return (
    <section className="space-y-4" aria-labelledby="product-variants-heading">
      <div>
        <h3 id="product-variants-heading" className="text-[13.5px] font-semibold text-foreground">
          Variants
        </h3>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          Manage the variants customers choose between and the price each one sells for.
        </p>
      </div>

      {detail.product.variants.length === 0 ? (
        <p className="rounded-lg border border-dashed border-border px-4 py-6 text-center text-[12px] text-muted-foreground">
          This product has no variants yet. Add the first one below.
        </p>
      ) : (
        <div className="overflow-hidden rounded-lg border border-border">
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead>Variant</TableHead>
                <TableHead>SKU</TableHead>
                <TableHead>Price</TableHead>
                <TableHead>Availability</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {detail.product.variants.map((variant) => {
                const prices = detail.pricesByVariant.get(variant.id) ?? [];
                const basePrice = prices.find((price) => price.price_list_id === null) ?? null;
                const available = detail.availabilityByVariant.get(variant.id) ?? null;
                const isEditing = editingId === variant.id;
                const isExpanded = expandedVariantId === variant.id;

                return (
                  <Fragment key={variant.id}>
                    <TableRow>
                      <TableCell className="min-w-[150px]">
                        {isEditing ? (
                          <div className="space-y-1.5">
                            <Label htmlFor={`variant-title-${variant.id}`} className="sr-only">
                              Variant title
                            </Label>
                            <Input
                              id={`variant-title-${variant.id}`}
                              value={draftTitle}
                              onChange={(event) => setDraftTitle(event.target.value)}
                              disabled={savingVariant}
                              className="h-8"
                              autoFocus
                            />
                          </div>
                        ) : (
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="text-[12.5px] font-medium text-foreground">
                              {variant.title}
                            </span>
                            <ProductStatusPill status={variant.status} />
                          </div>
                        )}
                      </TableCell>
                      <TableCell className="min-w-[120px]">
                        {isEditing ? (
                          <>
                            <Label htmlFor={`variant-sku-${variant.id}`} className="sr-only">
                              SKU
                            </Label>
                            <Input
                              id={`variant-sku-${variant.id}`}
                              value={draftSku}
                              onChange={(event) => setDraftSku(event.target.value)}
                              disabled={savingVariant}
                              className="h-8 font-mono"
                              placeholder="SKU"
                            />
                          </>
                        ) : (
                          <span className="font-mono text-[11px] text-ink-soft">
                            {variant.sku ?? "—"}
                          </span>
                        )}
                      </TableCell>
                      <TableCell>
                        {basePrice ? (
                          <span className="text-[12px] text-ink-soft tnum">
                            {formatMinorAmount(basePrice.amount, basePrice.currency_code)}
                          </span>
                        ) : (
                          <span className="text-[11.5px] text-muted-foreground">No price</span>
                        )}
                      </TableCell>
                      <TableCell>
                        {isEditing ? (
                          <div className="flex items-center gap-2">
                            <Switch
                              id={`variant-inventory-${variant.id}`}
                              checked={draftManageInventory}
                              onCheckedChange={setDraftManageInventory}
                              disabled={savingVariant}
                            />
                            <Label htmlFor={`variant-inventory-${variant.id}`}>
                              Track inventory
                            </Label>
                          </div>
                        ) : !variant.manage_inventory ? (
                          <Badge variant="outline">Not tracked</Badge>
                        ) : available === null ? (
                          <span className="text-[11.5px] text-muted-foreground">Unknown</span>
                        ) : (
                          <div className="flex items-center gap-2">
                            <StockPill available={available} />
                            <span className="font-mono text-[11px] text-muted-foreground tnum">
                              {available}
                            </span>
                          </div>
                        )}
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap items-center justify-end gap-1">
                          {isEditing ? (
                            <>
                              <Button
                                size="sm"
                                onClick={() => void saveVariant(variant)}
                                disabled={savingVariant}
                              >
                                {savingVariant ? (
                                  <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                                ) : (
                                  <Check className="size-3.5" aria-hidden="true" />
                                )}
                                Save
                              </Button>
                              <Button
                                size="sm"
                                variant="ghost"
                                onClick={cancelEdit}
                                disabled={savingVariant}
                              >
                                <X className="size-3.5" aria-hidden="true" />
                                Cancel
                              </Button>
                            </>
                          ) : (
                            <>
                              <Button
                                variant="outline"
                                size="sm"
                                onClick={() => setExpandedVariantId(isExpanded ? null : variant.id)}
                                aria-expanded={isExpanded}
                                aria-controls={`variant-prices-${variant.id}`}
                              >
                                {isExpanded ? (
                                  <ChevronUp className="size-3.5" aria-hidden="true" />
                                ) : (
                                  <ChevronDown className="size-3.5" aria-hidden="true" />
                                )}
                                Prices
                              </Button>
                              <Button
                                variant="ghost"
                                size="icon-sm"
                                onClick={() => startEdit(variant)}
                                aria-label={`Edit ${variant.title}`}
                              >
                                <Pencil className="size-3.5" aria-hidden="true" />
                              </Button>
                              <Button
                                variant="ghost"
                                size="icon-sm"
                                onClick={() => setPendingDelete(variant)}
                                aria-label={`Delete ${variant.title}`}
                              >
                                <Trash2 className="size-3.5" aria-hidden="true" />
                              </Button>
                            </>
                          )}
                        </div>
                      </TableCell>
                    </TableRow>
                    {isExpanded && !isEditing ? (
                      <TableRow className="hover:bg-transparent">
                        <TableCell colSpan={5} className="bg-secondary/30 p-4">
                          <VariantPriceEditor
                            id={`variant-prices-${variant.id}`}
                            variant={variant}
                            defaultCurrency={defaultCurrency}
                            priceLists={detail.priceLists}
                            onChanged={onChanged}
                          />
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </Fragment>
                );
              })}
            </TableBody>
          </Table>
        </div>
      )}

      <form
        className="rounded-xl border border-dashed border-border p-4"
        onSubmit={(event) => {
          event.preventDefault();
          void addVariant();
        }}
      >
        <p className="text-[12.5px] font-medium text-foreground">Add variant</p>
        <div className="mt-3 grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label htmlFor="new-variant-title">Title</Label>
            <Input
              id="new-variant-title"
              value={newTitle}
              onChange={(event) => setNewTitle(event.target.value)}
              placeholder="Large"
              disabled={addingVariant}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="new-variant-sku">SKU</Label>
            <Input
              id="new-variant-sku"
              value={newSku}
              onChange={(event) => setNewSku(event.target.value)}
              placeholder="RCC-LRG-01"
              className="font-mono"
              disabled={addingVariant}
            />
          </div>
        </div>
        <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Switch
              id="new-variant-inventory"
              checked={newManageInventory}
              onCheckedChange={setNewManageInventory}
              disabled={addingVariant}
            />
            <Label htmlFor="new-variant-inventory">Track inventory</Label>
          </div>
          <Button type="submit" size="sm" disabled={addingVariant || newTitle.trim().length === 0}>
            {addingVariant ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <Plus className="size-3.5" aria-hidden="true" />
            )}
            Add variant
          </Button>
        </div>
      </form>

      <Dialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open && !deletingVariant) setPendingDelete(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {pendingDelete?.title ?? "variant"}?</DialogTitle>
            <DialogDescription>
              Removing this variant also removes the prices attached to it. Orders that already
              reference it keep their historical records.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setPendingDelete(null)}
              disabled={deletingVariant}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void removeVariant()}
              disabled={deletingVariant}
            >
              {deletingVariant ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : null}
              Delete variant
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

function VariantPriceEditor({
  id,
  variant,
  defaultCurrency,
  priceLists,
  onChanged,
}: {
  id: string;
  variant: Variant;
  defaultCurrency: string;
  priceLists: PriceList[];
  onChanged: () => void;
}) {
  const { client } = useAuth();
  const [currency, setCurrency] = useState(() => normalizeCurrency(defaultCurrency));
  const normalizedCurrency = currency.trim().toUpperCase();
  const currencyValid = isValidCurrency(normalizedCurrency);

  const [prices, setPrices] = useState<Price[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);
  const [busy, setBusy] = useState(false);

  const [amount, setAmount] = useState("");
  const [editingPriceId, setEditingPriceId] = useState<string | null>(null);
  const [editAmount, setEditAmount] = useState("");
  const [pendingDelete, setPendingDelete] = useState<Price | null>(null);

  useEffect(() => {
    if (!currencyValid) {
      setPrices([]);
      setLoading(false);
      setLoadError(null);
      return;
    }
    if (!client) {
      setPrices([]);
      setLoading(false);
      setLoadError("No store is selected.");
      return;
    }

    const controller = new AbortController();
    setLoading(true);
    setLoadError(null);
    loadVariantPrices(client, variant.id, normalizedCurrency, controller.signal)
      .then((next) => {
        if (controller.signal.aborted) return;
        setPrices(next.filter((price) => price.currency_code.toUpperCase() === normalizedCurrency));
        setLoading(false);
      })
      .catch((cause) => {
        if (controller.signal.aborted) return;
        setLoadError(errorMessage(cause));
        setLoading(false);
      });

    return () => controller.abort();
  }, [client, variant.id, normalizedCurrency, currencyValid, version]);

  const priceListTitles = useMemo(
    () => new Map(priceLists.map((list) => [list.id, list.title])),
    [priceLists],
  );

  const hasBasePrice = prices.some(
    (price) =>
      price.price_list_id === null &&
      price.min_quantity === 1 &&
      price.customer_group_id === null &&
      price.sales_channel_id === null,
  );

  const createPrice = async () => {
    if (!client || busy || !currencyValid) return;
    const parsed = parsePriceAmount(amount);
    if (!parsed.ok) {
      toast.error(parsed.message);
      return;
    }

    setBusy(true);
    try {
      const created = await client.prices.create({
        variant_id: variant.id,
        currency_code: normalizedCurrency,
        amount: parsed.minor,
        price_list_id: null,
      });
      setPrices((current) => [...current, created]);
      setAmount("");
      toast.success("Price added.");
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const startEdit = (price: Price) => {
    setEditingPriceId(price.id);
    setEditAmount(formatMinorInput(price.amount));
  };

  const savePrice = async (price: Price) => {
    if (!client || busy) return;
    const parsed = parsePriceAmount(editAmount);
    if (!parsed.ok) {
      toast.error(parsed.message);
      return;
    }

    setBusy(true);
    try {
      const updated = await client.prices.update(price.id, { amount: parsed.minor });
      setPrices((current) => current.map((entry) => (entry.id === updated.id ? updated : entry)));
      setEditingPriceId(null);
      toast.success("Price updated.");
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const deletePrice = async () => {
    if (!client || pendingDelete === null || busy) return;
    setBusy(true);
    try {
      await client.prices.delete(pendingDelete.id);
      setPrices((current) => current.filter((entry) => entry.id !== pendingDelete.id));
      setPendingDelete(null);
      toast.success("Price deleted.");
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div id={id} className="space-y-4">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="space-y-1.5">
          <Label htmlFor={`price-currency-${variant.id}`}>Currency</Label>
          <Input
            id={`price-currency-${variant.id}`}
            value={currency}
            onChange={(event) => setCurrency(event.target.value.toUpperCase())}
            maxLength={3}
            autoComplete="off"
            spellCheck={false}
            disabled={busy}
            className="w-24 font-mono uppercase"
            aria-invalid={!currencyValid}
            aria-describedby={currencyValid ? undefined : `price-currency-error-${variant.id}`}
          />
        </div>
        <p className="text-[11px] text-muted-foreground">
          Enter amounts in major units, for example 19.99.
        </p>
      </div>

      {!currencyValid ? (
        <p
          id={`price-currency-error-${variant.id}`}
          role="alert"
          className="text-[11.5px] text-destructive"
        >
          Use a three-letter currency code such as USD.
        </p>
      ) : null}

      {!currencyValid ? null : loading ? (
        <div
          className="flex items-center gap-2 py-3 text-[11.5px] text-muted-foreground"
          aria-busy="true"
        >
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          Loading prices…
        </div>
      ) : loadError ? (
        <div
          role="alert"
          className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-destructive/20 bg-destructive/5 px-3 py-2 text-[11.5px] text-destructive"
        >
          <span>{loadError}</span>
          <Button size="sm" variant="outline" onClick={() => setVersion((value) => value + 1)}>
            Try again
          </Button>
        </div>
      ) : prices.length === 0 ? (
        <p className="rounded-md border border-dashed border-border px-3 py-4 text-center text-[11.5px] text-muted-foreground">
          No prices in {normalizedCurrency} yet.
        </p>
      ) : (
        <ul className="divide-y divide-border rounded-md border border-border bg-card">
          {prices.map((price) => (
            <li
              key={price.id}
              className="flex flex-wrap items-center justify-between gap-3 px-3 py-2.5"
            >
              <div className="min-w-0">
                <p className="text-[12.5px] font-medium text-foreground tnum">
                  {formatMinorAmount(price.amount, price.currency_code)}
                </p>
                <p className="mt-0.5 text-[10.5px] text-muted-foreground">
                  {price.price_list_id === null
                    ? "Base price"
                    : (priceListTitles.get(price.price_list_id) ?? "Price list")}
                  {price.min_quantity > 1 ? ` · quantity ${price.min_quantity}+` : ""}
                </p>
              </div>

              {editingPriceId === price.id ? (
                <div className="flex flex-wrap items-center gap-2">
                  <Label htmlFor={`price-amount-${price.id}`} className="sr-only">
                    Amount for {formatMinorAmount(price.amount, price.currency_code)}
                  </Label>
                  <Input
                    id={`price-amount-${price.id}`}
                    value={editAmount}
                    onChange={(event) => setEditAmount(event.target.value)}
                    inputMode="decimal"
                    autoComplete="off"
                    disabled={busy}
                    className="h-8 w-28"
                    autoFocus
                  />
                  <Button size="sm" onClick={() => void savePrice(price)} disabled={busy}>
                    {busy ? (
                      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                    ) : (
                      <Check className="size-3.5" aria-hidden="true" />
                    )}
                    Save
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setEditingPriceId(null)}
                    disabled={busy}
                  >
                    <X className="size-3.5" aria-hidden="true" />
                    Cancel
                  </Button>
                </div>
              ) : (
                <div className="flex items-center gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => startEdit(price)}
                    aria-label={`Edit ${formatMinorAmount(price.amount, price.currency_code)}`}
                  >
                    <Pencil className="size-3.5" aria-hidden="true" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setPendingDelete(price)}
                    aria-label={`Delete ${formatMinorAmount(price.amount, price.currency_code)}`}
                  >
                    <Trash2 className="size-3.5" aria-hidden="true" />
                  </Button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}

      {currencyValid && !loading && !loadError ? (
        hasBasePrice ? (
          <p className="text-[11.5px] text-muted-foreground">
            A base price already exists for {normalizedCurrency}. Edit it above to change the
            amount.
          </p>
        ) : (
          <form
            className="flex flex-wrap items-end gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              void createPrice();
            }}
          >
            <div className="space-y-1.5">
              <Label htmlFor={`new-price-${variant.id}`}>New base price</Label>
              <Input
                id={`new-price-${variant.id}`}
                value={amount}
                onChange={(event) => setAmount(event.target.value)}
                inputMode="decimal"
                autoComplete="off"
                placeholder="19.99"
                disabled={busy}
                className="w-32"
              />
            </div>
            <Button type="submit" size="sm" disabled={busy || amount.trim().length === 0}>
              {busy ? (
                <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
              ) : (
                <Plus className="size-3.5" aria-hidden="true" />
              )}
              Add price
            </Button>
          </form>
        )
      ) : null}

      <Dialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setPendingDelete(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              Delete{" "}
              {pendingDelete
                ? formatMinorAmount(pendingDelete.amount, pendingDelete.currency_code)
                : "price"}
              ?
            </DialogTitle>
            <DialogDescription>
              This price will no longer be available to the storefront. This cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingDelete(null)} disabled={busy}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void deletePrice()} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              Delete price
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
