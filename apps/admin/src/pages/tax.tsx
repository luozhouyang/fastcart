import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  ChevronDown,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import type { TaxCategory, TaxRate, TaxRegion } from "@fastcart/shared-types";

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
import { errorMessage, fetchAll } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { PERMISSIONS } from "@/lib/permissions";
import { consumePendingCommand, onCommand } from "@/lib/commands";
import { demoTaxCategories, demoTaxRates, demoTaxRegions } from "@/lib/demo-operations";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  formatRateInput,
  formatRatePercent,
  normalizeCountryCode,
  normalizeProvince,
  parsePercentToBps,
} from "@/lib/operations";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70";

interface TaxData {
  categories: TaxCategory[];
  regions: TaxRegion[];
  rates: TaxRate[];
}

interface RateDraft {
  regionId: string;
  categoryId: string | null;
  name: string;
  rateBps: number;
  isInclusive: boolean;
  isDefault: boolean;
}

export function TaxPage() {
  const { mode, client, store, dataKey, can } = useAuth();
  const [demoCategories, setDemoCategories] = useState<TaxCategory[]>(demoTaxCategories);
  const [demoRegions, setDemoRegions] = useState<TaxRegion[]>(demoTaxRegions);
  const [demoRates, setDemoRates] = useState<TaxRate[]>(demoTaxRates);

  const [categoryDialogOpen, setCategoryDialogOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<TaxCategory | null>(null);
  const [deleteCategory, setDeleteCategory] = useState<TaxCategory | null>(null);
  const [categoryBusy, setCategoryBusy] = useState(false);

  const [regionDialogOpen, setRegionDialogOpen] = useState(false);
  const [editingRegion, setEditingRegion] = useState<TaxRegion | null>(null);
  const [deleteRegion, setDeleteRegion] = useState<TaxRegion | null>(null);
  const [regionBusy, setRegionBusy] = useState(false);

  const [rateDialogOpen, setRateDialogOpen] = useState(false);
  const [editingRate, setEditingRate] = useState<TaxRate | null>(null);
  const [deleteRate, setDeleteRate] = useState<TaxRate | null>(null);
  const [rateBusy, setRateBusy] = useState(false);

  const loader = useCallback(
    (signal: AbortSignal): Promise<TaxData> => {
      if (mode === "demo") {
        return Promise.resolve({
          categories: demoCategories,
          regions: demoRegions,
          rates: demoRates,
        });
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return Promise.all([
        fetchAll((query) => client.tax.categories.list(query, { signal })),
        fetchAll((query) => client.tax.regions.list(query, { signal })),
      ]).then(async ([categories, regions]) => {
        const groups = await Promise.all(
          regions.map((region) => client.tax.rates.list({ region_id: region.id }, { signal })),
        );
        return { categories, regions, rates: groups.flat() };
      });
    },
    [mode, client, demoCategories, demoRegions, demoRates],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    client,
    store?.id,
    dataKey,
    demoCategories,
    demoRegions,
    demoRates,
  ]);

  useEffect(() => {
    setCategoryDialogOpen(false);
    setEditingCategory(null);
    setDeleteCategory(null);
    setRegionDialogOpen(false);
    setEditingRegion(null);
    setDeleteRegion(null);
    setRateDialogOpen(false);
    setEditingRate(null);
    setDeleteRate(null);
  }, [store?.id, dataKey]);

  useEffect(() => {
    const pending = consumePendingCommand();
    if (pending === "new-tax-rate") openCreateRate();
    return onCommand((command) => {
      if (command === "new-tax-rate") openCreateRate();
    });
  }, []);

  const openCreateCategory = useCallback(() => {
    setEditingCategory(null);
    setCategoryDialogOpen(true);
  }, []);

  const openEditCategory = useCallback((category: TaxCategory) => {
    setEditingCategory(category);
    setCategoryDialogOpen(true);
  }, []);

  const openCreateRegion = useCallback(() => {
    setEditingRegion(null);
    setRegionDialogOpen(true);
  }, []);

  const openEditRegion = useCallback((region: TaxRegion) => {
    setEditingRegion(region);
    setRegionDialogOpen(true);
  }, []);

  const openCreateRate = useCallback(() => {
    setEditingRate(null);
    setRateDialogOpen(true);
  }, []);

  const openEditRate = useCallback((rate: TaxRate) => {
    setEditingRate(rate);
    setRateDialogOpen(true);
  }, []);

  const saveCategory = useCallback(
    async (name: string, code: string, description: string): Promise<void> => {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingCategory) {
          setDemoCategories((current) =>
            current.map((category) =>
              category.id === editingCategory.id
                ? {
                    ...category,
                    name,
                    code,
                    description: description.length === 0 ? null : description,
                    updated_at: now,
                  }
                : category,
            ),
          );
          toast.success("Tax category updated in the demo");
        } else {
          setDemoCategories((current) => [
            ...current,
            {
              id: `txcat_demo_${Date.now()}`,
              name,
              code,
              description: description.length === 0 ? null : description,
              created_at: now,
              updated_at: now,
            },
          ]);
          toast.success("Tax category created in the demo");
        }
        return;
      }
      if (!client) throw new Error("No store is selected.");
      if (editingCategory) {
        await client.tax.categories.update(editingCategory.id, {
          name,
          code,
          description: description.length === 0 ? null : description,
        });
        toast.success("Tax category updated");
      } else {
        await client.tax.categories.create({
          name,
          code,
          description: description.length === 0 ? null : description,
        });
        toast.success("Tax category created");
      }
      reload();
    },
    [mode, client, editingCategory, reload],
  );

  const saveRegion = useCallback(
    async (name: string, countryCode: string, provinceCode: string | null): Promise<void> => {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingRegion) {
          setDemoRegions((current) =>
            current.map((region) =>
              region.id === editingRegion.id
                ? {
                    ...region,
                    name,
                    country_code: countryCode,
                    province_code: provinceCode,
                    updated_at: now,
                  }
                : region,
            ),
          );
          toast.success("Tax region updated in the demo");
        } else {
          setDemoRegions((current) => [
            ...current,
            {
              id: `txreg_demo_${Date.now()}`,
              name,
              country_code: countryCode,
              province_code: provinceCode,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
          ]);
          toast.success("Tax region created in the demo");
        }
        return;
      }
      if (!client) throw new Error("No store is selected.");
      if (editingRegion) {
        await client.tax.regions.update(editingRegion.id, {
          name,
          country_code: countryCode,
          province_code: provinceCode,
        });
        toast.success("Tax region updated");
      } else {
        await client.tax.regions.create({
          name,
          country_code: countryCode,
          province_code: provinceCode,
        });
        toast.success("Tax region created");
      }
      reload();
    },
    [mode, client, editingRegion, reload],
  );

  const saveRate = useCallback(
    async (draft: RateDraft): Promise<void> => {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingRate) {
          setDemoRates((current) =>
            current.map((rate) =>
              rate.id === editingRate.id
                ? {
                    ...rate,
                    tax_category_id: draft.categoryId,
                    name: draft.name,
                    rate_bps: draft.rateBps,
                    is_inclusive: draft.isInclusive,
                    is_default: draft.isDefault,
                    updated_at: now,
                  }
                : draft.isDefault && rate.tax_region_id === editingRate.tax_region_id
                  ? { ...rate, is_default: false }
                  : rate,
            ),
          );
          toast.success("Tax rate updated in the demo");
        } else {
          setDemoRates((current) => [
            ...current.map((rate) =>
              draft.isDefault && rate.tax_region_id === draft.regionId
                ? { ...rate, is_default: false }
                : rate,
            ),
            {
              id: `txrate_demo_${Date.now()}`,
              tax_region_id: draft.regionId,
              tax_category_id: draft.categoryId,
              name: draft.name,
              rate_bps: draft.rateBps,
              is_inclusive: draft.isInclusive,
              is_default: draft.isDefault,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
          ]);
          toast.success("Tax rate created in the demo");
        }
        return;
      }
      if (!client) throw new Error("No store is selected.");
      if (editingRate) {
        await client.tax.rates.update(editingRate.id, {
          category_id: draft.categoryId,
          name: draft.name,
          rate_bps: draft.rateBps,
          is_inclusive: draft.isInclusive,
          is_default: draft.isDefault,
        });
        toast.success("Tax rate updated");
      } else {
        await client.tax.rates.create({
          region_id: draft.regionId,
          category_id: draft.categoryId,
          name: draft.name,
          rate_bps: draft.rateBps,
          is_inclusive: draft.isInclusive,
          is_default: draft.isDefault,
        });
        toast.success("Tax rate created");
      }
      reload();
    },
    [mode, client, editingRate, reload],
  );

  const confirmDeleteCategory = useCallback(async () => {
    if (!deleteCategory || categoryBusy) return;
    const target = deleteCategory;
    setCategoryBusy(true);
    try {
      if (mode === "demo") {
        setDemoCategories((current) => current.filter((category) => category.id !== target.id));
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.tax.categories.delete(target.id);
      }
      setDeleteCategory(null);
      toast.success("Tax category deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCategoryBusy(false);
    }
  }, [deleteCategory, categoryBusy, mode, client, reload]);

  const confirmDeleteRegion = useCallback(async () => {
    if (!deleteRegion || regionBusy) return;
    const target = deleteRegion;
    setRegionBusy(true);
    try {
      if (mode === "demo") {
        setDemoRegions((current) => current.filter((region) => region.id !== target.id));
        setDemoRates((current) => current.filter((rate) => rate.tax_region_id !== target.id));
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.tax.regions.delete(target.id);
      }
      setDeleteRegion(null);
      toast.success("Tax region deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRegionBusy(false);
    }
  }, [deleteRegion, regionBusy, mode, client, reload]);

  const confirmDeleteRate = useCallback(async () => {
    if (!deleteRate || rateBusy) return;
    const target = deleteRate;
    setRateBusy(true);
    try {
      if (mode === "demo") {
        setDemoRates((current) => current.filter((rate) => rate.id !== target.id));
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.tax.rates.delete(target.id);
      }
      setDeleteRate(null);
      toast.success("Tax rate deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRateBusy(false);
    }
  }, [deleteRate, rateBusy, mode, client, reload]);

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const { categories, regions, rates } = data;
  const categoryName = (categoryId: string | null) =>
    categoryId === null
      ? "No category"
      : (categories.find((category) => category.id === categoryId)?.name ?? categoryId);
  const regionName = (regionId: string) =>
    regions.find((region) => region.id === regionId)?.name ?? regionId;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Tax"
        description="Set up tax categories, regional tax rules and the rates applied to each product category."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              disabled={loading}
              aria-label="Refresh tax settings"
              title="Refresh tax settings"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            {can(PERMISSIONS.settings.write) ? (
              <Button onClick={openCreateRate} disabled={regions.length === 0}>
                <Plus className="size-4" aria-hidden="true" />
                New rate
              </Button>
            ) : null}
          </>
        }
      />

      {regions.length === 0 ? (
        <Card className="px-6 py-4 text-[12px] text-muted-foreground">
          Create a tax region before adding rates.
        </Card>
      ) : null}

      <Card className="overflow-hidden">
        <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Tax categories</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Products reference a category; rates are defined per category within a region.
            </p>
          </div>
          {can(PERMISSIONS.settings.write) ? (
            <Button variant="outline" size="sm" onClick={openCreateCategory}>
              <Plus className="size-3.5" aria-hidden="true" />
              New category
            </Button>
          ) : null}
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Category</TableHead>
              <TableHead className="w-[140px]">Code</TableHead>
              <TableHead className="hidden md:table-cell">Description</TableHead>
              <TableHead className="hidden w-[110px] lg:table-cell">Rates</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {categories.map((category) => (
              <TableRow key={category.id}>
                <TableCell className="pl-6">
                  <span className="text-[13px] font-medium text-foreground">{category.name}</span>
                  <p className="mt-1 font-mono text-[10.5px] text-faint">{category.id}</p>
                </TableCell>
                <TableCell>
                  <code className="rounded-md border border-border bg-secondary/60 px-2 py-1 font-mono text-[11px] text-ink-soft">
                    {category.code}
                  </code>
                </TableCell>
                <TableCell className="hidden max-w-[320px] truncate text-[12px] text-muted-foreground md:table-cell">
                  {category.description ?? "—"}
                </TableCell>
                <TableCell className="hidden text-[12px] text-ink-soft tnum lg:table-cell">
                  {rates.filter((rate) => rate.tax_category_id === category.id).length}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Actions for ${category.name}`}
                      >
                        <MoreHorizontal className="size-4" aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-[190px]">
                      <DropdownMenuLabel className="truncate">{category.name}</DropdownMenuLabel>
                      <DropdownMenuItem onSelect={() => openEditCategory(category)}>
                        <Pencil aria-hidden="true" />
                        Edit category
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        variant="destructive"
                        onSelect={() => setDeleteCategory(category)}
                      >
                        <Trash2 aria-hidden="true" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
            {categories.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={5} className="p-0">
                  <EmptyState
                    title="No tax categories"
                    description="Create a category such as standard or reduced before assigning rates."
                    action={
                      <Button onClick={openCreateCategory}>
                        <Plus className="size-4" aria-hidden="true" />
                        New category
                      </Button>
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </Card>

      <Card className="overflow-hidden">
        <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Tax regions</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              A tax region groups the rates that apply to one country or province.
            </p>
          </div>
          {can(PERMISSIONS.settings.write) ? (
            <Button variant="outline" size="sm" onClick={openCreateRegion}>
              <Plus className="size-3.5" aria-hidden="true" />
              New tax region
            </Button>
          ) : null}
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Region</TableHead>
              <TableHead className="w-[110px]">Country</TableHead>
              <TableHead className="w-[130px]">Province</TableHead>
              <TableHead className="hidden w-[110px] md:table-cell">Rates</TableHead>
              <TableHead className="hidden w-[120px] lg:table-cell">Created</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {regions.map((region) => (
              <TableRow key={region.id}>
                <TableCell className="pl-6">
                  <span className="text-[13px] font-medium text-foreground">{region.name}</span>
                  <p className="mt-1 font-mono text-[10.5px] text-faint">{region.id}</p>
                </TableCell>
                <TableCell className="font-mono text-[12px] text-ink-soft">
                  {region.country_code}
                </TableCell>
                <TableCell className="font-mono text-[12px] text-ink-soft">
                  {region.province_code ?? "—"}
                </TableCell>
                <TableCell className="hidden text-[12px] text-ink-soft tnum md:table-cell">
                  {rates.filter((rate) => rate.tax_region_id === region.id).length}
                </TableCell>
                <TableCell className="hidden text-muted-foreground lg:table-cell">
                  {shortDate(fromUnixSeconds(region.created_at))}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Actions for ${region.name}`}
                      >
                        <MoreHorizontal className="size-4" aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-[190px]">
                      <DropdownMenuLabel className="truncate">{region.name}</DropdownMenuLabel>
                      <DropdownMenuItem onSelect={() => openEditRegion(region)}>
                        <Pencil aria-hidden="true" />
                        Edit region
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        variant="destructive"
                        onSelect={() => setDeleteRegion(region)}
                      >
                        <Trash2 aria-hidden="true" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
            {regions.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={6} className="p-0">
                  <EmptyState
                    title="No tax regions"
                    description="Add the country or province where you collect tax."
                    action={
                      <Button onClick={openCreateRegion}>
                        <Plus className="size-4" aria-hidden="true" />
                        New tax region
                      </Button>
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </Card>

      <Card className="overflow-hidden">
        <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Tax rates</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              A region has one default rate; category-specific rates take precedence.
            </p>
          </div>
          <p className="text-[11.5px] text-muted-foreground">
            {rates.length} {rates.length === 1 ? "rate" : "rates"}
            {mode === "demo" ? " · sample data" : ""}
          </p>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Rate</TableHead>
              <TableHead className="hidden w-[140px] md:table-cell">Region</TableHead>
              <TableHead className="hidden w-[140px] lg:table-cell">Category</TableHead>
              <TableHead className="w-[90px]">Rate</TableHead>
              <TableHead className="w-[100px]">Pricing</TableHead>
              <TableHead className="w-[90px]">Default</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rates.map((rate) => (
              <TableRow key={rate.id}>
                <TableCell className="pl-6">
                  <span className="text-[13px] font-medium text-foreground">{rate.name}</span>
                  <p className="mt-1 font-mono text-[10.5px] text-faint">{rate.id}</p>
                </TableCell>
                <TableCell className="hidden text-[12px] text-ink-soft md:table-cell">
                  {regionName(rate.tax_region_id)}
                </TableCell>
                <TableCell className="hidden text-[12px] text-ink-soft lg:table-cell">
                  {categoryName(rate.tax_category_id)}
                </TableCell>
                <TableCell className="font-medium text-foreground tnum">
                  {formatRatePercent(rate.rate_bps)}
                </TableCell>
                <TableCell>
                  <Badge variant={rate.is_inclusive ? "info" : "secondary"}>
                    {rate.is_inclusive ? "Inclusive" : "Exclusive"}
                  </Badge>
                </TableCell>
                <TableCell>
                  {rate.is_default ? (
                    <Badge variant="brand">Default</Badge>
                  ) : (
                    <span className="text-[12px] text-faint">—</span>
                  )}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Actions for ${rate.name}`}
                      >
                        <MoreHorizontal className="size-4" aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-[190px]">
                      <DropdownMenuLabel className="truncate">{rate.name}</DropdownMenuLabel>
                      <DropdownMenuItem onSelect={() => openEditRate(rate)}>
                        <Pencil aria-hidden="true" />
                        Edit rate
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem variant="destructive" onSelect={() => setDeleteRate(rate)}>
                        <Trash2 aria-hidden="true" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
            {rates.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={7} className="p-0">
                  <EmptyState
                    title="No tax rates"
                    description="Add a rate to start calculating tax for a region."
                    action={
                      can(PERMISSIONS.settings.write) ? (
                        <Button onClick={openCreateRate} disabled={regions.length === 0}>
                          <Plus className="size-4" aria-hidden="true" />
                          New rate
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

      <CategoryDialog
        open={categoryDialogOpen}
        onOpenChange={(open) => {
          setCategoryDialogOpen(open);
          if (!open) setEditingCategory(null);
        }}
        category={editingCategory}
        onSubmit={saveCategory}
      />

      <TaxRegionDialog
        open={regionDialogOpen}
        onOpenChange={(open) => {
          setRegionDialogOpen(open);
          if (!open) setEditingRegion(null);
        }}
        region={editingRegion}
        onSubmit={saveRegion}
      />

      <RateDialog
        open={rateDialogOpen}
        onOpenChange={(open) => {
          setRateDialogOpen(open);
          if (!open) setEditingRate(null);
        }}
        rate={editingRate}
        regions={regions}
        categories={categories}
        onSubmit={saveRate}
      />

      <ConfirmDialog
        open={deleteCategory !== null}
        title="Delete tax category?"
        description={`“${deleteCategory?.name}” can only be deleted when no tax rate or product references it. The API error is shown if it is still in use.`}
        busy={categoryBusy}
        onCancel={() => setDeleteCategory(null)}
        onConfirm={() => void confirmDeleteCategory()}
      />

      <ConfirmDialog
        open={deleteRegion !== null}
        title="Delete tax region?"
        description={`“${deleteRegion?.name}” and its ${rates.filter((rate) => rate.tax_region_id === deleteRegion?.id).length} rate(s) will be removed. Regions linked from a store region may still reference it.`}
        busy={regionBusy}
        onCancel={() => setDeleteRegion(null)}
        onConfirm={() => void confirmDeleteRegion()}
      />

      <ConfirmDialog
        open={deleteRate !== null}
        title="Delete tax rate?"
        description={`“${deleteRate?.name}” will no longer apply to its region.`}
        busy={rateBusy}
        onCancel={() => setDeleteRate(null)}
        onConfirm={() => void confirmDeleteRate()}
      />
    </div>
  );
}

function SelectField({
  id,
  label,
  value,
  onChange,
  disabled,
  children,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <div className="relative">
        <select
          id={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          disabled={disabled}
          className={selectClass}
        >
          {children}
        </select>
        <ChevronDown
          className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
          aria-hidden="true"
        />
      </div>
    </div>
  );
}

function CategoryDialog({
  open,
  onOpenChange,
  category,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  category: TaxCategory | null;
  onSubmit: (name: string, code: string, description: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [code, setCode] = useState("");
  const [description, setDescription] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);
  const [codeError, setCodeError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(category?.name ?? "");
    setCode(category?.code ?? "");
    setDescription(category?.description ?? "");
    setNameError(null);
    setCodeError(null);
    setBusy(false);
  }, [open, category?.id]);

  const submit = async () => {
    if (busy) return;
    const trimmedName = name.trim();
    const trimmedCode = code.trim();
    if (trimmedName.length === 0) {
      setNameError("Enter a name for this category.");
      return;
    }
    if (trimmedCode.length === 0) {
      setCodeError("Enter a stable code, for example standard.");
      return;
    }
    setBusy(true);
    try {
      await onSubmit(trimmedName, trimmedCode, description.trim());
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
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{category ? "Edit tax category" : "New tax category"}</DialogTitle>
            <DialogDescription>
              Products are assigned a category so the right rate can be resolved.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="category-name">Name</Label>
            <Input
              id="category-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setNameError(null);
              }}
              placeholder="Standard"
              autoComplete="off"
              disabled={busy}
              aria-invalid={nameError !== null}
            />
            {nameError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {nameError}
              </p>
            ) : null}
          </div>

          <div className="space-y-2">
            <Label htmlFor="category-code">Code</Label>
            <Input
              id="category-code"
              value={code}
              onChange={(event) => {
                setCode(event.target.value);
                setCodeError(null);
              }}
              placeholder="standard"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              aria-invalid={codeError !== null}
              className="font-mono"
            />
            <p className="text-[11px] text-faint">Unique per store; used by integrations.</p>
            {codeError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {codeError}
              </p>
            ) : null}
          </div>

          <div className="space-y-2">
            <Label htmlFor="category-description">Description</Label>
            <textarea
              id="category-description"
              rows={3}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              disabled={busy}
              placeholder="Default rate for most physical goods"
              className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs transition-[color,box-shadow,border-color] outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
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
              {category ? "Save changes" : "Create category"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function TaxRegionDialog({
  open,
  onOpenChange,
  region,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  region: TaxRegion | null;
  onSubmit: (name: string, countryCode: string, provinceCode: string | null) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [country, setCountry] = useState("");
  const [province, setProvince] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);
  const [countryError, setCountryError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(region?.name ?? "");
    setCountry(region?.country_code ?? "");
    setProvince(region?.province_code ?? "");
    setNameError(null);
    setCountryError(null);
    setBusy(false);
  }, [open, region?.id]);

  const submit = async () => {
    if (busy) return;
    const trimmedName = name.trim();
    if (trimmedName.length === 0) {
      setNameError("Enter a name for this tax region.");
      return;
    }
    const countryCode = normalizeCountryCode(country);
    if (countryCode === null) {
      setCountryError("Enter a two-letter ISO country code, for example US.");
      return;
    }
    setBusy(true);
    try {
      await onSubmit(trimmedName, countryCode, normalizeProvince(province));
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
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{region ? "Edit tax region" : "New tax region"}</DialogTitle>
            <DialogDescription>
              Tax rates are calculated for the country and optional province of a tax region.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="tax-region-name">Name</Label>
            <Input
              id="tax-region-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setNameError(null);
              }}
              placeholder="United States"
              autoComplete="off"
              disabled={busy}
              aria-invalid={nameError !== null}
            />
            {nameError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {nameError}
              </p>
            ) : null}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="tax-region-country">Country</Label>
              <Input
                id="tax-region-country"
                value={country}
                onChange={(event) => {
                  setCountry(event.target.value.toUpperCase().slice(0, 2));
                  setCountryError(null);
                }}
                placeholder="US"
                maxLength={2}
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                aria-invalid={countryError !== null}
                className="font-mono uppercase"
              />
              {countryError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {countryError}
                </p>
              ) : null}
            </div>
            <div className="space-y-2">
              <Label htmlFor="tax-region-province">Province / state</Label>
              <Input
                id="tax-region-province"
                value={province}
                onChange={(event) => setProvince(event.target.value.toUpperCase())}
                placeholder="Optional"
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                className="font-mono uppercase"
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
              {region ? "Save changes" : "Create region"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function RateDialog({
  open,
  onOpenChange,
  rate,
  regions,
  categories,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  rate: TaxRate | null;
  regions: TaxRegion[];
  categories: TaxCategory[];
  onSubmit: (draft: RateDraft) => Promise<void>;
}) {
  const [regionId, setRegionId] = useState("");
  const [categoryId, setCategoryId] = useState("");
  const [name, setName] = useState("");
  const [percent, setPercent] = useState("");
  const [isInclusive, setIsInclusive] = useState(false);
  const [isDefault, setIsDefault] = useState(false);
  const [nameError, setNameError] = useState<string | null>(null);
  const [percentError, setPercentError] = useState<string | null>(null);
  const [regionError, setRegionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setRegionId(rate?.tax_region_id ?? regions[0]?.id ?? "");
    setCategoryId(rate?.tax_category_id ?? "");
    setName(rate?.name ?? "");
    setPercent(rate ? formatRateInput(rate.rate_bps) : "");
    setIsInclusive(rate?.is_inclusive ?? false);
    setIsDefault(rate?.is_default ?? false);
    setNameError(null);
    setPercentError(null);
    setRegionError(null);
    setBusy(false);
  }, [open, rate?.id, regions]);

  const submit = async () => {
    if (busy) return;
    if (regionId.length === 0) {
      setRegionError("Choose a tax region.");
      return;
    }
    const trimmedName = name.trim();
    if (trimmedName.length === 0) {
      setNameError("Enter a name for this rate.");
      return;
    }
    const parsed = parsePercentToBps(percent);
    if (!parsed.ok) {
      setPercentError(parsed.message);
      return;
    }
    setBusy(true);
    try {
      await onSubmit({
        regionId,
        categoryId: categoryId.length === 0 ? null : categoryId,
        name: trimmedName,
        rateBps: parsed.bps,
        isInclusive,
        isDefault,
      });
      onOpenChange(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
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
            <DialogTitle>{rate ? "Edit tax rate" : "New tax rate"}</DialogTitle>
            <DialogDescription>
              Percentages are stored as basis points by the API. A region can have one default rate,
              plus category-specific rates.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              id="tax-rate-region"
              label="Region"
              value={regionId}
              onChange={(value) => {
                setRegionId(value);
                setRegionError(null);
              }}
              disabled={busy || rate !== null}
            >
              {regions.map((region) => (
                <option key={region.id} value={region.id}>
                  {region.name}
                </option>
              ))}
            </SelectField>
            <SelectField
              id="tax-rate-category"
              label="Category"
              value={categoryId}
              onChange={setCategoryId}
              disabled={busy}
            >
              <option value="">No category</option>
              {categories.map((category) => (
                <option key={category.id} value={category.id}>
                  {category.name}
                </option>
              ))}
            </SelectField>
          </div>
          {regionError ? (
            <p className="-mt-2 text-[11.5px] text-destructive" role="alert">
              {regionError}
            </p>
          ) : null}

          <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_120px]">
            <div className="space-y-2">
              <Label htmlFor="tax-rate-name">Name</Label>
              <Input
                id="tax-rate-name"
                value={name}
                onChange={(event) => {
                  setName(event.target.value);
                  setNameError(null);
                }}
                placeholder="US sales tax"
                autoComplete="off"
                disabled={busy}
                aria-invalid={nameError !== null}
              />
              {nameError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {nameError}
                </p>
              ) : null}
            </div>
            <div className="space-y-2">
              <Label htmlFor="tax-rate-percent">Rate</Label>
              <div className="relative">
                <Input
                  id="tax-rate-percent"
                  type="number"
                  min="0"
                  max="100"
                  step="0.01"
                  inputMode="decimal"
                  value={percent}
                  onChange={(event) => {
                    setPercent(event.target.value);
                    setPercentError(null);
                  }}
                  placeholder="8.25"
                  disabled={busy}
                  aria-invalid={percentError !== null}
                  className="pr-8"
                />
                <span className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-[12px] text-faint">
                  %
                </span>
              </div>
              {percentError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {percentError}
                </p>
              ) : null}
            </div>
          </div>

          <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3.5">
            <div>
              <Label htmlFor="tax-rate-inclusive">Tax inclusive</Label>
              <p className="mt-1 text-[11.5px] text-muted-foreground">
                Prices already include this tax; the tax portion is extracted.
              </p>
            </div>
            <Switch
              id="tax-rate-inclusive"
              checked={isInclusive}
              onCheckedChange={setIsInclusive}
              disabled={busy}
            />
          </div>

          <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3.5">
            <div>
              <Label htmlFor="tax-rate-default">Default rate</Label>
              <p className="mt-1 text-[11.5px] text-muted-foreground">
                Used when no category-specific rate matches. Replaces the region's previous default.
              </p>
            </div>
            <Switch
              id="tax-rate-default"
              checked={isDefault}
              onCheckedChange={setIsDefault}
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
              {rate ? "Save changes" : "Create rate"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ConfirmDialog({
  open,
  title,
  description,
  busy,
  onCancel,
  onConfirm,
}: {
  open: boolean;
  title: string;
  description: string;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !busy) onCancel();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button variant="destructive" onClick={onConfirm} disabled={busy}>
            {busy ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Trash2 className="size-4" aria-hidden="true" />
            )}
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
