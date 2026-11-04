import { useCallback, useEffect, useMemo, useState } from "react";
import { Check, ChevronDown, Download, Loader2, Search, SlidersHorizontal, X } from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/page-header";
import { ProductThumb } from "@/components/product-thumb";
import { ProgressBar } from "@/components/progress-bar";
import { StockPill } from "@/components/status-pill";
import { ErrorState, TableSkeleton } from "@/components/states";
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
import { demoInventory } from "@/lib/demo";
import { downloadCsv } from "@/lib/format";
import { adjustStock, loadInventory } from "@/lib/loaders";
import type { InventoryLocationView, InventoryRowView } from "@/lib/models";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

type StockFilter = "all" | "low" | "out";

const FILTERS: Array<{ value: StockFilter; label: string }> = [
  { value: "all", label: "All" },
  { value: "low", label: "Low stock" },
  { value: "out", label: "Out of stock" },
];

function available(row: InventoryRowView): number {
  return row.stocked - row.reserved;
}

export function InventoryPage() {
  const { mode, client, store, dataKey } = useAuth();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<StockFilter>("all");
  const [locationId, setLocationId] = useState<string | null>(null);
  const [deltas, setDeltas] = useState<Record<string, number>>({});
  const [adjustOpen, setAdjustOpen] = useState(false);
  const [preset, setPreset] = useState<InventoryRowView | null>(null);

  useEffect(() => {
    setDeltas({});
    setLocationId(null);
    setFilter("all");
    setQuery("");
  }, [store?.id, dataKey]);

  useEffect(() => {
    const pending = consumePendingCommand();
    if (pending === "adjust-stock") {
      setPreset(null);
      setAdjustOpen(true);
    }
    return onCommand((command) => {
      if (command === "adjust-stock") {
        setPreset(null);
        setAdjustOpen(true);
      }
    });
  }, []);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") return Promise.resolve(demoInventory());
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadInventory(client, signal);
    },
    [mode, client],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);

  const rows = useMemo(
    () =>
      (data?.rows ?? []).map((row) => {
        const delta = deltas[row.id] ?? 0;
        return delta === 0 ? row : { ...row, stocked: row.stocked + delta };
      }),
    [data?.rows, deltas],
  );

  const locations = useMemo<InventoryLocationView[]>(() => {
    if (!data) return [];
    return data.locations.map((location) => {
      const locationRows = rows.filter((row) => row.locationId === location.id);
      return {
        ...location,
        units: locationRows.reduce((total, row) => total + row.stocked, 0),
        items: new Set(locationRows.map((row) => row.inventoryItemId)).size,
      };
    });
  }, [data, rows]);

  const totalUnits = rows.reduce((total, row) => total + row.stocked, 0);
  const lowCount = rows.filter((row) => available(row) <= 10).length;
  const outCount = rows.filter((row) => available(row) <= 0).length;
  const maxUnits = Math.max(1, ...locations.map((location) => location.units));
  const maxStocked = Math.max(1, ...rows.map((row) => row.stocked));

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return rows.filter((row) => {
      if (filter === "low" && available(row) > 10) return false;
      if (filter === "out" && available(row) > 0) return false;
      if (locationId !== null && row.locationId !== locationId) return false;
      if (needle.length === 0) return true;
      return (
        row.product.toLowerCase().includes(needle) ||
        row.sku.toLowerCase().includes(needle) ||
        row.location.toLowerCase().includes(needle)
      );
    });
  }, [rows, filter, locationId, query]);

  const submitAdjustment = async (row: InventoryRowView, delta: number, reason: string) => {
    if (delta === 0) {
      toast.error("Enter a quantity change other than zero.");
      return;
    }
    if (mode === "demo") {
      setDeltas((current) => ({ ...current, [row.id]: (current[row.id] ?? 0) + delta }));
      toast.success(`${row.product} adjusted by ${delta > 0 ? "+" : ""}${delta}`);
      return;
    }
    if (!client) throw new Error("No store is selected.");
    await adjustStock(client, row, delta, reason);
    toast.success(`${row.product} adjusted by ${delta > 0 ? "+" : ""}${delta}`);
    reload();
  };

  const openAdjust = (row: InventoryRowView | null) => {
    setPreset(row);
    setAdjustOpen(true);
  };

  const exportInventory = () => {
    downloadCsv(
      "fastcart-inventory.csv",
      ["Product", "SKU", "Location", "Stocked", "Reserved", "Available", "Incoming"],
      filtered.map((row) => [
        row.product,
        row.sku,
        row.location,
        row.stocked,
        row.reserved,
        available(row),
        row.incoming,
      ]),
    );
  };

  if (!data && loading) return <TableSkeleton rows={6} />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const activeLocation = locations.find((location) => location.id === locationId);

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Inventory"
        description={`${totalUnits.toLocaleString("en-US")} units across ${locations.length} ${locations.length === 1 ? "location" : "locations"} · ${lowCount} ${lowCount === 1 ? "item needs" : "items need"} attention`}
        actions={
          <>
            <Button variant="outline" onClick={exportInventory} disabled={filtered.length === 0}>
              <Download className="size-4" aria-hidden="true" />
              Export
            </Button>
            <Button onClick={() => openAdjust(null)}>
              <SlidersHorizontal className="size-4" aria-hidden="true" />
              Adjust stock
            </Button>
          </>
        }
      />

      <section className="grid gap-4 md:grid-cols-2 xl:grid-cols-3" aria-label="Locations">
        {locations.map((location) => (
          <button
            key={location.id}
            type="button"
            onClick={() =>
              setLocationId((current) => (current === location.id ? null : location.id))
            }
            aria-pressed={locationId === location.id}
            className={cn(
              "cursor-pointer rounded-xl border bg-card p-5 text-left shadow-soft transition-colors duration-150 hover:border-line-strong focus-visible:ring-[3px] focus-visible:ring-ring/30 focus-visible:outline-none",
              locationId === location.id ? "border-brand/40" : "border-border",
            )}
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="truncate text-[13px] font-medium text-foreground">{location.name}</p>
                <p className="mt-1 truncate text-[11px] text-muted-foreground">
                  {location.address}
                </p>
              </div>
              {location.primary ? (
                <span className="shrink-0 rounded-full border border-brand/20 bg-brand-soft px-2 py-0.5 text-[10px] font-medium text-brand-ink">
                  Primary
                </span>
              ) : null}
            </div>

            <div className="mt-5 flex items-baseline gap-1.5">
              <span className="text-[32px] leading-none font-medium tracking-[-0.02em] text-foreground tnum">
                {location.units.toLocaleString("en-US")}
              </span>
              <span className="text-[11px] text-muted-foreground">units</span>
              <span className="ml-auto text-[11px] text-muted-foreground">
                {location.items} {location.items === 1 ? "item" : "items"}
              </span>
            </div>
            <ProgressBar
              className="mt-3 h-1"
              value={location.units}
              max={maxUnits}
              tone={location.units === 0 ? "destructive" : "brand"}
            />
          </button>
        ))}
      </section>

      <div className="flex flex-col gap-3 xl:flex-row xl:items-center">
        <div className="relative w-full xl:w-[280px]">
          <Search
            className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
            aria-hidden="true"
          />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search products or SKUs"
            className="pr-9 pl-9"
            aria-label="Search inventory"
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
          {FILTERS.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => setFilter(option.value)}
              aria-pressed={filter === option.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center gap-1.5 rounded-full px-3 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                filter === option.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
              <span className="font-mono text-[10px] text-faint tnum">
                {option.value === "all"
                  ? rows.length
                  : option.value === "low"
                    ? lowCount
                    : outCount}
              </span>
            </button>
          ))}
        </div>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" className="shrink-0 justify-between gap-2">
              {activeLocation?.name ?? "All locations"}
              <ChevronDown className="size-3.5 text-faint" aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-[200px]">
            <DropdownMenuLabel>Location</DropdownMenuLabel>
            <DropdownMenuItem onSelect={() => setLocationId(null)}>
              All locations
              {locationId === null ? (
                <Check className="ml-auto size-3.5 text-brand" aria-hidden="true" />
              ) : null}
            </DropdownMenuItem>
            {locations.map((location) => (
              <DropdownMenuItem key={location.id} onSelect={() => setLocationId(location.id)}>
                {location.name}
                {locationId === location.id ? (
                  <Check className="ml-auto size-3.5 text-brand" aria-hidden="true" />
                ) : null}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Stock by location</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Available is stocked minus reserved
            </p>
          </div>
          <span className="text-[11px] text-faint">Updated just now</span>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Product</TableHead>
              <TableHead className="w-[160px]">Location</TableHead>
              <TableHead className="w-[190px]">Available</TableHead>
              <TableHead className="hidden w-[82px] text-right sm:table-cell">Reserved</TableHead>
              <TableHead className="hidden w-[82px] text-right md:table-cell">Incoming</TableHead>
              <TableHead className="w-[108px]">Status</TableHead>
              <TableHead className="w-[76px] pr-6 text-right" aria-label="Adjust" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {filtered.map((row) => {
              const onHand = available(row);
              return (
                <TableRow key={row.id} className="h-[62px]">
                  <TableCell className="pl-6">
                    <div className="flex items-center gap-3">
                      <ProductThumb
                        title={row.product}
                        swatch={row.swatch}
                        className="size-8 shrink-0 rounded-md"
                        initialClassName="text-[13px]"
                      />
                      <div className="min-w-0">
                        <p className="truncate text-[12.5px] font-medium text-foreground">
                          {row.product}
                        </p>
                        <p className="mt-0.5 truncate font-mono text-[10px] text-faint">
                          {row.sku}
                        </p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell className="text-[12px] text-muted-foreground">
                    {row.location}
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-3">
                      <span className="w-7 shrink-0 text-right font-mono text-[12px] text-ink-soft tnum">
                        {onHand}
                      </span>
                      <ProgressBar
                        className="h-1 max-w-[82px]"
                        value={row.stocked}
                        max={maxStocked}
                        tone={onHand <= 0 ? "destructive" : onHand <= 10 ? "warning" : "brand"}
                      />
                      <span className="font-mono text-[10px] whitespace-nowrap text-faint">
                        {row.stocked} stocked
                      </span>
                    </div>
                  </TableCell>
                  <TableCell className="hidden text-right font-mono text-[11.5px] text-ink-soft tnum sm:table-cell">
                    {row.reserved}
                  </TableCell>
                  <TableCell className="hidden text-right font-mono text-[11.5px] text-ink-soft tnum md:table-cell">
                    {row.incoming}
                  </TableCell>
                  <TableCell>
                    <StockPill available={onHand} />
                  </TableCell>
                  <TableCell className="pr-6 text-right">
                    <button
                      type="button"
                      onClick={() => openAdjust(row)}
                      className="cursor-pointer text-[11.5px] text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                    >
                      Adjust
                    </button>
                  </TableCell>
                </TableRow>
              );
            })}
            {filtered.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={7} className="py-14 text-center">
                  <p className="text-[13px] font-medium text-foreground">No stock rows found</p>
                  <p className="mt-1 text-[12px] text-muted-foreground">
                    Adjust the filters or search for a different product.
                  </p>
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>

        <div className="flex flex-col gap-2 border-t border-border px-6 py-3.5 text-[11.5px] text-muted-foreground sm:flex-row sm:items-center sm:justify-between">
          <span>Reserved stock is held for open checkouts</span>
          <span className="font-mono text-[10.5px] text-faint">
            {filtered.length} of {rows.length} rows
          </span>
        </div>
      </Card>

      <AdjustStockDialog
        open={adjustOpen}
        onOpenChange={setAdjustOpen}
        rows={rows}
        preset={preset}
        onSubmit={submitAdjustment}
      />
    </div>
  );
}

function AdjustStockDialog({
  open,
  onOpenChange,
  rows,
  preset,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  rows: InventoryRowView[];
  preset: InventoryRowView | null;
  onSubmit: (row: InventoryRowView, delta: number, reason: string) => Promise<void>;
}) {
  const [rowId, setRowId] = useState("");
  const [quantity, setQuantity] = useState("");
  const [reason, setReason] = useState("Manual adjustment");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setRowId(preset?.id ?? rows[0]?.id ?? "");
    setQuantity("");
    setReason("Manual adjustment");
    // Reset only when the dialog opens; typing must not be interrupted by row refreshes.
  }, [open, preset?.id]);

  const selected = rows.find((row) => row.id === rowId) ?? preset ?? null;

  const submit = async () => {
    if (!selected || busy) return;
    const delta = Number(quantity);
    if (!Number.isInteger(delta) || delta === 0) {
      toast.error("Enter a whole number other than zero.");
      return;
    }
    setBusy(true);
    try {
      await onSubmit(
        selected,
        delta,
        reason.trim().length > 0 ? reason.trim() : "Manual adjustment",
      );
      onOpenChange(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Adjust stock</DialogTitle>
          <DialogDescription>
            Add or remove units at one location. Reserved units are not changed.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="stock-row">Inventory item</Label>
            <div className="relative">
              <select
                id="stock-row"
                value={rowId}
                onChange={(event) => setRowId(event.target.value)}
                disabled={busy || preset !== null}
                className="h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70"
              >
                {rows.map((row) => (
                  <option key={row.id} value={row.id}>
                    {row.product} · {row.location} ({available(row)} available)
                  </option>
                ))}
              </select>
              <ChevronDown
                className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                aria-hidden="true"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="stock-delta">Quantity change</Label>
            <Input
              id="stock-delta"
              type="number"
              step="1"
              placeholder="+12 or -3"
              value={quantity}
              onChange={(event) => setQuantity(event.target.value)}
              disabled={busy}
            />
            <p className="text-[11px] text-faint">
              Use a negative number to remove stock, for example −3.
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="stock-reason">Reason</Label>
            <Input
              id="stock-reason"
              value={reason}
              onChange={(event) => setReason(event.target.value)}
              disabled={busy}
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={busy}>
            {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
            Apply adjustment
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
