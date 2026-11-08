import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  Check,
  ChevronDown,
  Copy,
  Globe2,
  KeyRound,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  ShieldAlert,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";
import type { CreatedPublishableApiKey, PublishableApiKey } from "@fastcart/admin-sdk";
import type { Region, SalesChannel, TaxRegion } from "@fastcart/shared-types";

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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Switch } from "@/components/ui/switch";
import { errorMessage, fetchAll } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { consumePendingCommand, onCommand } from "@/lib/commands";
import {
  demoPublishableKeys,
  demoRegions,
  demoSalesChannels,
  demoTaxRegions,
} from "@/lib/demo-operations";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  addCountryCodes,
  maskPublishableKey,
  parsePaymentProviders,
  summarizeCountryCodes,
} from "@/lib/operations";
import { isValidCurrency, normalizeCurrency } from "@/lib/pricing";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70";

const DEFAULT_CURRENCY = "USD";

interface RegionDraft {
  name: string;
  currencyCode: string;
  countries: string[];
  taxRegionId: string | null;
  paymentProviders: string[];
  isDefault: boolean;
}

export function RegionsPage() {
  const { mode, client, store, dataKey } = useAuth();
  const [demoRegionList, setDemoRegionList] = useState<Region[]>(demoRegions);
  const [demoKeyList, setDemoKeyList] = useState<PublishableApiKey[]>(demoPublishableKeys);

  const [regionDialogOpen, setRegionDialogOpen] = useState(false);
  const [editingRegion, setEditingRegion] = useState<Region | null>(null);
  const [deleteRegion, setDeleteRegion] = useState<Region | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);

  const [keyDialogOpen, setKeyDialogOpen] = useState(false);
  const [createdKey, setCreatedKey] = useState<CreatedPublishableApiKey | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<PublishableApiKey | null>(null);
  const [revokeBusy, setRevokeBusy] = useState(false);
  const [channels, setChannels] = useState<SalesChannel[] | null>(null);
  const [channelsLoading, setChannelsLoading] = useState(false);
  const [channelsError, setChannelsError] = useState<string | null>(null);

  const storeCurrency = normalizeCurrency(store?.currency_code, DEFAULT_CURRENCY);

  const regionLoader = useCallback(
    (signal: AbortSignal): Promise<{ regions: Region[]; taxRegions: TaxRegion[] }> => {
      if (mode === "demo") {
        return Promise.resolve({
          regions: [...demoRegionList].sort((a, b) => b.created_at - a.created_at),
          taxRegions: demoTaxRegions(),
        });
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return Promise.all([
        fetchAll((query) => client.regions.list(query, { signal })),
        fetchAll((query) => client.tax.regions.list(query, { signal })),
      ]).then(([regions, taxRegions]) => ({ regions, taxRegions }));
    },
    [mode, client, demoRegionList],
  );

  const {
    data,
    error,
    loading,
    reload: reloadRegions,
  } = useAsyncData(regionLoader, [mode, client, store?.id, dataKey, demoRegionList]);

  const keyLoader = useCallback(
    (signal: AbortSignal): Promise<PublishableApiKey[]> => {
      if (mode === "demo") return Promise.resolve(demoKeyList);
      if (!client) return Promise.reject(new Error("No store is selected."));
      return client.regions.publishableKeys.list({ signal });
    },
    [mode, client, demoKeyList],
  );

  const {
    data: keys,
    error: keysError,
    loading: keysLoading,
    reload: reloadKeys,
  } = useAsyncData(keyLoader, [mode, client, store?.id, dataKey, demoKeyList]);

  useEffect(() => {
    setRegionDialogOpen(false);
    setEditingRegion(null);
    setDeleteRegion(null);
    setKeyDialogOpen(false);
    setCreatedKey(null);
    setRevokeTarget(null);
  }, [store?.id, dataKey]);

  useEffect(() => {
    const pending = consumePendingCommand();
    if (pending === "new-region") openCreateRegion();
    return onCommand((command) => {
      if (command === "new-region") openCreateRegion();
    });
  }, []);

  useEffect(() => {
    if (!keyDialogOpen) return;
    if (mode === "demo") {
      setChannels(demoSalesChannels());
      setChannelsLoading(false);
      setChannelsError(null);
      return;
    }
    if (!client) return;
    const controller = new AbortController();
    setChannels(null);
    setChannelsLoading(true);
    setChannelsError(null);
    fetchAll((query) => client.salesChannels.list(query, { signal: controller.signal }))
      .then((result) => {
        if (!controller.signal.aborted) setChannels(result);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setChannelsError(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setChannelsLoading(false);
      });
    return () => controller.abort();
  }, [keyDialogOpen, mode, client]);

  const openCreateRegion = useCallback(() => {
    setEditingRegion(null);
    setRegionDialogOpen(true);
  }, []);

  const openEditRegion = useCallback((region: Region) => {
    setEditingRegion(region);
    setRegionDialogOpen(true);
  }, []);

  const saveRegion = useCallback(
    async (draft: RegionDraft): Promise<void> => {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingRegion) {
          setDemoRegionList((current) =>
            current.map((region) => {
              if (region.id === editingRegion.id) {
                return {
                  ...region,
                  name: draft.name,
                  currency_code: draft.currencyCode,
                  countries: draft.countries,
                  tax_region_id: draft.taxRegionId,
                  payment_providers: draft.paymentProviders,
                  is_default: draft.isDefault,
                  updated_at: now,
                };
              }
              return draft.isDefault ? { ...region, is_default: false } : region;
            }),
          );
          toast.success("Region updated in the demo");
        } else {
          const created: Region = {
            id: `reg_demo_${Date.now()}`,
            name: draft.name,
            currency_code: draft.currencyCode,
            countries: draft.countries,
            tax_region_id: draft.taxRegionId,
            payment_providers: draft.paymentProviders,
            is_default: draft.isDefault,
            metadata: {},
            created_at: now,
            updated_at: now,
          };
          setDemoRegionList((current) => [
            created,
            ...(draft.isDefault
              ? current.map((region) => ({ ...region, is_default: false }))
              : current),
          ]);
          toast.success("Region created in the demo");
        }
        return;
      }

      if (!client) throw new Error("No store is selected.");
      const body = {
        name: draft.name,
        currency_code: draft.currencyCode,
        countries: draft.countries,
        tax_region_id: draft.taxRegionId,
        payment_providers: draft.paymentProviders,
        is_default: draft.isDefault,
      };
      if (editingRegion) {
        await client.regions.update(editingRegion.id, body);
        toast.success("Region updated");
      } else {
        await client.regions.create(body);
        toast.success("Region created");
      }
      reloadRegions();
    },
    [mode, client, editingRegion, reloadRegions],
  );

  const confirmDeleteRegion = useCallback(async () => {
    if (!deleteRegion || deleteBusy) return;
    const target = deleteRegion;
    setDeleteBusy(true);
    try {
      if (mode === "demo") {
        setDemoRegionList((current) => current.filter((region) => region.id !== target.id));
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.regions.delete(target.id);
      }
      setDeleteRegion(null);
      toast.success("Region deleted");
      reloadRegions();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDeleteBusy(false);
    }
  }, [deleteRegion, deleteBusy, mode, client, reloadRegions]);

  const createKey = useCallback(
    async (title: string, channelIds: string[]): Promise<void> => {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        const token = `pk_demo_${Math.random().toString(36).slice(2, 14)}`;
        const created: CreatedPublishableApiKey = {
          id: `pk_demo_${Date.now()}`,
          title,
          token_prefix: token.slice(0, 12),
          sales_channel_ids: channelIds,
          revoked_at: null,
          last_used_at: null,
          created_at: now,
          token,
        };
        const { token: _secret, ...record } = created;
        setDemoKeyList((current) => [record, ...current]);
        setCreatedKey(created);
        setKeyDialogOpen(false);
        toast.success("Publishable key created in the demo");
        return;
      }

      if (!client) throw new Error("No store is selected.");
      const created = await client.regions.publishableKeys.create({
        title,
        sales_channel_ids: channelIds,
      });
      setCreatedKey(created);
      setKeyDialogOpen(false);
      toast.success("Publishable key created");
      reloadKeys();
    },
    [mode, client, reloadKeys],
  );

  const confirmRevoke = useCallback(async () => {
    if (!revokeTarget || revokeBusy) return;
    const target = revokeTarget;
    setRevokeBusy(true);
    try {
      if (mode === "demo") {
        setDemoKeyList((current) =>
          current.map((key) =>
            key.id === target.id ? { ...key, revoked_at: Math.floor(Date.now() / 1000) } : key,
          ),
        );
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.regions.publishableKeys.revoke(target.id);
      }
      setRevokeTarget(null);
      toast.success("Publishable key revoked");
      reloadKeys();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRevokeBusy(false);
    }
  }, [revokeTarget, revokeBusy, mode, client, reloadKeys]);

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reloadRegions} />;
  if (!data) return null;

  const regionRows = data.regions;
  const taxRegions = data.taxRegions;
  const taxRegionName = (id: string | null) =>
    id === null ? null : (taxRegions.find((region) => region.id === id)?.name ?? null);

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Regions"
        description="Group countries by currency, link them to tax regions and choose the payment providers available at checkout."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reloadRegions}
              disabled={loading}
              aria-label="Refresh regions"
              title="Refresh regions"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            <Button onClick={openCreateRegion}>
              <Plus className="size-4" aria-hidden="true" />
              New region
            </Button>
          </>
        }
      />

      <Card className="overflow-hidden">
        <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Store regions</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              The default region is used when a storefront request does not resolve a country.
            </p>
          </div>
          <p className="text-[11.5px] text-muted-foreground">
            {regionRows.length} {regionRows.length === 1 ? "region" : "regions"}
            {mode === "demo" ? " · sample data" : ""}
          </p>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Region</TableHead>
              <TableHead className="w-[90px]">Currency</TableHead>
              <TableHead className="w-[180px]">Countries</TableHead>
              <TableHead className="hidden w-[160px] lg:table-cell">Tax region</TableHead>
              <TableHead className="hidden w-[190px] xl:table-cell">Payment providers</TableHead>
              <TableHead className="hidden w-[120px] md:table-cell">Updated</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {regionRows.map((region) => {
              const linkedTaxRegion = taxRegionName(region.tax_region_id);
              return (
                <TableRow key={region.id}>
                  <TableCell className="pl-6">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-[13px] font-medium text-foreground">{region.name}</span>
                      {region.is_default ? <Badge variant="brand">Default</Badge> : null}
                    </div>
                    <p className="mt-1 font-mono text-[10.5px] text-faint">{region.id}</p>
                  </TableCell>
                  <TableCell className="font-mono text-[12px] text-ink-soft">
                    {region.currency_code}
                  </TableCell>
                  <TableCell>
                    <span className="text-[12px] text-ink-soft" title={region.countries.join(", ")}>
                      {summarizeCountryCodes(region.countries)}
                    </span>
                  </TableCell>
                  <TableCell className="hidden lg:table-cell">
                    {linkedTaxRegion ? (
                      <span className="text-[12px] text-ink-soft">{linkedTaxRegion}</span>
                    ) : (
                      <span className="text-[12px] text-faint">Not linked</span>
                    )}
                  </TableCell>
                  <TableCell className="hidden xl:table-cell">
                    {region.payment_providers.length === 0 ? (
                      <span className="text-[12px] text-faint">None</span>
                    ) : (
                      <div className="flex flex-wrap gap-1">
                        {region.payment_providers.map((provider) => (
                          <Badge key={provider} variant="outline">
                            {provider}
                          </Badge>
                        ))}
                      </div>
                    )}
                  </TableCell>
                  <TableCell className="hidden text-muted-foreground md:table-cell">
                    {shortDate(fromUnixSeconds(region.updated_at))}
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
              );
            })}
            {regionRows.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={7} className="p-0">
                  <EmptyState
                    title="No regions yet"
                    description="Create a region so customers can browse prices in their currency and pay with an enabled provider."
                    action={
                      <Button onClick={openCreateRegion}>
                        <Plus className="size-4" aria-hidden="true" />
                        New region
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
        <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-4">
          <div>
            <p className="flex items-center gap-2 text-[13.5px] font-medium text-foreground">
              <KeyRound className="size-4 text-faint" aria-hidden="true" />
              Publishable API keys
            </p>
            <p className="mt-1 max-w-xl text-[11.5px] leading-relaxed text-muted-foreground">
              Storefront catalog requests are authorized with a publishable key scoped to one or
              more sales channels. The full key is shown once, at creation.
            </p>
          </div>
          <Button variant="outline" onClick={() => setKeyDialogOpen(true)}>
            <Plus className="size-4" aria-hidden="true" />
            Create key
          </Button>
        </div>

        {keysError ? (
          <div className="border-t border-border px-6 py-8 text-center">
            <p className="text-[12.5px] text-destructive">{keysError}</p>
            <Button variant="outline" size="sm" className="mt-3" onClick={reloadKeys}>
              <RefreshCw className="size-3.5" aria-hidden="true" />
              Try again
            </Button>
          </div>
        ) : keys === null ? (
          <div className="border-t border-border px-6 py-8 text-center text-[12.5px] text-muted-foreground">
            {keysLoading ? "Loading keys…" : "No key data."}
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-6">Key</TableHead>
                <TableHead className="w-[150px]">Prefix</TableHead>
                <TableHead className="hidden w-[170px] md:table-cell">Sales channels</TableHead>
                <TableHead className="w-[110px]">Status</TableHead>
                <TableHead className="hidden w-[120px] lg:table-cell">Created</TableHead>
                <TableHead className="hidden w-[120px] xl:table-cell">Last used</TableHead>
                <TableHead className="w-10 pr-5" aria-label="Actions" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {keys.map((key) => {
                const revoked = key.revoked_at !== null;
                return (
                  <TableRow key={key.id}>
                    <TableCell className="pl-6 text-[13px] font-medium text-foreground">
                      {key.title}
                    </TableCell>
                    <TableCell>
                      <code className="rounded-md border border-border bg-secondary/60 px-2 py-1 font-mono text-[11px] text-ink-soft">
                        {maskPublishableKey(key.token_prefix)}
                      </code>
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <div className="flex flex-wrap gap-1">
                        {key.sales_channel_ids.map((channelId) => (
                          <Badge key={channelId} variant="secondary">
                            {channelId}
                          </Badge>
                        ))}
                      </div>
                    </TableCell>
                    <TableCell>
                      {revoked ? (
                        <Badge variant="destructive">Revoked</Badge>
                      ) : (
                        <Badge variant="brand">Active</Badge>
                      )}
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground lg:table-cell">
                      {shortDate(fromUnixSeconds(key.created_at))}
                    </TableCell>
                    <TableCell className="hidden text-muted-foreground xl:table-cell">
                      {key.last_used_at === null
                        ? "Never"
                        : shortDate(fromUnixSeconds(key.last_used_at))}
                    </TableCell>
                    <TableCell className="pr-5 text-right">
                      {revoked ? null : (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-destructive hover:text-destructive"
                          onClick={() => setRevokeTarget(key)}
                        >
                          Revoke
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                );
              })}
              {keys.length === 0 ? (
                <TableRow className="hover:bg-transparent">
                  <TableCell colSpan={7} className="p-0">
                    <EmptyState
                      title="No publishable keys"
                      description="Create a key before pointing a storefront at this store. Requests without one cannot read the catalog."
                      action={
                        <Button variant="outline" onClick={() => setKeyDialogOpen(true)}>
                          <Plus className="size-4" aria-hidden="true" />
                          Create key
                        </Button>
                      }
                    />
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        )}
      </Card>

      <RegionDialog
        open={regionDialogOpen}
        onOpenChange={(open) => {
          setRegionDialogOpen(open);
          if (!open) setEditingRegion(null);
        }}
        region={editingRegion}
        taxRegions={taxRegions}
        defaultCurrency={storeCurrency}
        onSubmit={saveRegion}
      />

      <KeyCreateDialog
        open={keyDialogOpen}
        onOpenChange={setKeyDialogOpen}
        channels={channels}
        loading={channelsLoading}
        error={channelsError}
        onSubmit={createKey}
      />

      <KeyRevealDialog created={createdKey} onClose={() => setCreatedKey(null)} />

      <Dialog
        open={deleteRegion !== null}
        onOpenChange={(open) => {
          if (!open && !deleteBusy) setDeleteRegion(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete region?</DialogTitle>
            <DialogDescription>
              “{deleteRegion?.name}” will be removed. Orders and carts that already reference this
              region keep their recorded values, but new checkouts cannot use it.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteRegion(null)} disabled={deleteBusy}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void confirmDeleteRegion()}
              disabled={deleteBusy}
            >
              {deleteBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Delete region
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={revokeTarget !== null}
        onOpenChange={(open) => {
          if (!open && !revokeBusy) setRevokeTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Revoke publishable key?</DialogTitle>
            <DialogDescription>
              “{revokeTarget?.title}” will stop authorizing storefront requests immediately. This
              cannot be undone; create a new key to restore access.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRevokeTarget(null)} disabled={revokeBusy}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void confirmRevoke()}
              disabled={revokeBusy}
            >
              {revokeBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <ShieldAlert className="size-4" aria-hidden="true" />
              )}
              Revoke key
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
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

function RegionDialog({
  open,
  onOpenChange,
  region,
  taxRegions,
  defaultCurrency,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  region: Region | null;
  taxRegions: TaxRegion[];
  defaultCurrency: string;
  onSubmit: (draft: RegionDraft) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [currency, setCurrency] = useState(defaultCurrency);
  const [countries, setCountries] = useState<string[]>([]);
  const [countryInput, setCountryInput] = useState("");
  const [countryError, setCountryError] = useState<string | null>(null);
  const [taxRegionId, setTaxRegionId] = useState("");
  const [providers, setProviders] = useState("");
  const [isDefault, setIsDefault] = useState(false);
  const [nameError, setNameError] = useState<string | null>(null);
  const [currencyError, setCurrencyError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(region?.name ?? "");
    setCurrency(region?.currency_code ?? defaultCurrency);
    setCountries(region?.countries ?? []);
    setCountryInput("");
    setCountryError(null);
    setTaxRegionId(region?.tax_region_id ?? "");
    setProviders(region?.payment_providers.join(", ") ?? "");
    setIsDefault(region?.is_default ?? false);
    setNameError(null);
    setCurrencyError(null);
    setBusy(false);
  }, [open, region?.id, defaultCurrency]);

  const addCountries = () => {
    const result = addCountryCodes(countries, countryInput);
    setCountries(result.codes);
    if (result.invalid.length > 0) {
      setCountryInput(result.invalid.join(", "));
      setCountryError(
        `Use two-letter ISO codes such as US or CA. Not valid: ${result.invalid.join(", ")}.`,
      );
      return false;
    }
    setCountryInput("");
    setCountryError(null);
    return true;
  };

  const removeCountry = (code: string) => {
    setCountries((current) => current.filter((entry) => entry !== code));
    setCountryError(null);
  };

  const submit = async () => {
    if (busy) return;
    const trimmedName = name.trim();
    if (trimmedName.length === 0) {
      setNameError("Enter a name for this region.");
      return;
    }
    if (!isValidCurrency(currency)) {
      setCurrencyError("Enter a three-letter ISO currency code, for example USD.");
      return;
    }
    const parsed = addCountryCodes(countries, countryInput);
    if (parsed.invalid.length > 0) {
      setCountryInput(parsed.invalid.join(", "));
      setCountryError(
        `Use two-letter ISO codes such as US or CA. Not valid: ${parsed.invalid.join(", ")}.`,
      );
      return;
    }
    if (parsed.codes.length === 0) {
      setCountryError("Add at least one country.");
      return;
    }

    setBusy(true);
    try {
      await onSubmit({
        name: trimmedName,
        currencyCode: normalizeCurrency(currency),
        countries: parsed.codes,
        taxRegionId: taxRegionId.length === 0 ? null : taxRegionId,
        paymentProviders: parsePaymentProviders(providers),
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
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-xl">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{region ? "Edit region" : "New region"}</DialogTitle>
            <DialogDescription>
              {region
                ? "Update the currency, countries and checkout providers for this region."
                : "A region sets the currency and countries served by one storefront market."}
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_120px]">
            <div className="space-y-2">
              <Label htmlFor="region-name">Name</Label>
              <Input
                id="region-name"
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
            <div className="space-y-2">
              <Label htmlFor="region-currency">Currency</Label>
              <Input
                id="region-currency"
                value={currency}
                onChange={(event) => {
                  setCurrency(event.target.value.toUpperCase().slice(0, 3));
                  setCurrencyError(null);
                }}
                placeholder="USD"
                maxLength={3}
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                aria-invalid={currencyError !== null}
                className="font-mono uppercase"
              />
              {currencyError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {currencyError}
                </p>
              ) : null}
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="region-countries">Countries</Label>
            <div className="flex flex-wrap items-center gap-1.5 rounded-md border border-input bg-card px-2 py-2 shadow-xs focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/25">
              {countries.map((code) => (
                <span
                  key={code}
                  className="inline-flex items-center gap-1 rounded-md border border-border bg-secondary px-1.5 py-1 font-mono text-[11px] text-ink-soft"
                >
                  {code}
                  <button
                    type="button"
                    onClick={() => removeCountry(code)}
                    disabled={busy}
                    aria-label={`Remove ${code}`}
                    className="cursor-pointer rounded-sm text-faint hover:text-foreground"
                  >
                    <X className="size-3" aria-hidden="true" />
                  </button>
                </span>
              ))}
              <input
                id="region-countries"
                value={countryInput}
                onChange={(event) => {
                  setCountryInput(event.target.value);
                  setCountryError(null);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === ",") {
                    event.preventDefault();
                    addCountries();
                  } else if (event.key === "Backspace" && countryInput.length === 0) {
                    setCountries((current) => current.slice(0, -1));
                  }
                }}
                onBlur={() => {
                  if (countryInput.trim().length > 0) addCountries();
                }}
                placeholder={countries.length === 0 ? "US, CA, GB" : "Add country"}
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                className="h-7 min-w-[120px] flex-1 bg-transparent font-mono text-[12.5px] text-foreground uppercase outline-none placeholder:font-sans placeholder:text-faint"
              />
            </div>
            <p className="text-[11px] leading-relaxed text-faint">
              Two-letter ISO 3166-1 codes, separated by commas or spaces. Press Enter to add.
            </p>
            {countryError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {countryError}
              </p>
            ) : null}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              id="region-tax"
              label="Tax region"
              value={taxRegionId}
              onChange={setTaxRegionId}
              disabled={busy}
            >
              <option value="">No tax region</option>
              {taxRegions.map((entry) => (
                <option key={entry.id} value={entry.id}>
                  {entry.name}
                </option>
              ))}
            </SelectField>
            <div className="space-y-2">
              <Label htmlFor="region-providers">Payment providers</Label>
              <Input
                id="region-providers"
                value={providers}
                onChange={(event) => setProviders(event.target.value)}
                placeholder="manual, stripe"
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                className="font-mono"
              />
            </div>
          </div>
          <p className="-mt-2 text-[11px] leading-relaxed text-faint">
            Payment provider identifiers are comma-separated. Leave empty when the region does not
            accept online payments.
          </p>

          <div className="flex items-start justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3.5">
            <div>
              <Label htmlFor="region-default">Default region</Label>
              <p className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">
                Only one region can be the default. Enabling this clears the previous default.
              </p>
            </div>
            <Switch
              id="region-default"
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
              {region ? "Save changes" : "Create region"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function KeyCreateDialog({
  open,
  onOpenChange,
  channels,
  loading,
  error,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  channels: SalesChannel[] | null;
  loading: boolean;
  error: string | null;
  onSubmit: (title: string, channelIds: string[]) => Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [titleError, setTitleError] = useState<string | null>(null);
  const [channelError, setChannelError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const initialized = useRef(false);

  useEffect(() => {
    if (!open) return;
    initialized.current = false;
    setTitle("");
    setSelected([]);
    setTitleError(null);
    setChannelError(null);
    setBusy(false);
  }, [open]);

  useEffect(() => {
    if (!open || channels === null || initialized.current) return;
    initialized.current = true;
    const first = channels.find((channel) => !channel.is_disabled);
    if (first) setSelected([first.id]);
  }, [open, channels]);

  const toggleChannel = (channelId: string) => {
    setSelected((current) =>
      current.includes(channelId)
        ? current.filter((entry) => entry !== channelId)
        : [...current, channelId],
    );
    setChannelError(null);
  };

  const submit = async () => {
    if (busy) return;
    const trimmedTitle = title.trim();
    if (trimmedTitle.length === 0) {
      setTitleError("Give this key a name so it can be identified later.");
      return;
    }
    if (selected.length === 0) {
      setChannelError("Select at least one sales channel.");
      return;
    }
    setBusy(true);
    try {
      await onSubmit(trimmedTitle, selected);
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
            <DialogTitle>Create publishable key</DialogTitle>
            <DialogDescription>
              The key is scoped to the sales channels you select. You will see the full secret once,
              immediately after creation.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="key-title">Name</Label>
            <Input
              id="key-title"
              value={title}
              onChange={(event) => {
                setTitle(event.target.value);
                setTitleError(null);
              }}
              placeholder="Storefront (production)"
              autoComplete="off"
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
            <Label>Sales channels</Label>
            {loading ? (
              <p className="rounded-lg border border-border bg-secondary/40 px-3.5 py-3 text-[12px] text-muted-foreground">
                Loading sales channels…
              </p>
            ) : error ? (
              <p className="rounded-lg border border-border bg-secondary/40 px-3.5 py-3 text-[12px] text-destructive">
                {error}
              </p>
            ) : channels === null || channels.length === 0 ? (
              <p className="rounded-lg border border-border bg-secondary/40 px-3.5 py-3 text-[12px] text-muted-foreground">
                No sales channels are available. Create one before issuing a key.
              </p>
            ) : (
              <div className="space-y-1.5">
                {channels.map((channel) => {
                  const checked = selected.includes(channel.id);
                  return (
                    <button
                      key={channel.id}
                      type="button"
                      role="checkbox"
                      aria-checked={checked}
                      disabled={busy || channel.is_disabled}
                      onClick={() => toggleChannel(channel.id)}
                      className={cn(
                        "flex w-full cursor-pointer items-center gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors",
                        checked
                          ? "border-brand/40 bg-brand-soft/40"
                          : "border-border bg-background hover:border-line-strong",
                        channel.is_disabled && "cursor-not-allowed opacity-50",
                      )}
                    >
                      <span
                        className={cn(
                          "flex size-4 shrink-0 items-center justify-center rounded-[4px] border",
                          checked
                            ? "border-primary bg-primary text-primary-foreground"
                            : "border-line-strong bg-card",
                        )}
                        aria-hidden="true"
                      >
                        {checked ? <Check className="size-3" strokeWidth={3} /> : null}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block text-[12.5px] font-medium text-foreground">
                          {channel.name}
                        </span>
                        <span className="mt-0.5 block font-mono text-[10.5px] text-faint">
                          {channel.id}
                        </span>
                      </span>
                      {channel.is_disabled ? <Badge variant="secondary">Disabled</Badge> : null}
                    </button>
                  );
                })}
              </div>
            )}
            {channelError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {channelError}
              </p>
            ) : null}
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
            <Button type="submit" disabled={busy || loading || error !== null}>
              {busy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <KeyRound className="size-4" aria-hidden="true" />
              )}
              Create key
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function KeyRevealDialog({
  created,
  onClose,
}: {
  created: CreatedPublishableApiKey | null;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (created) setCopied(false);
  }, [created?.id]);

  const copy = async () => {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.token);
      setCopied(true);
      toast.success("Key copied to the clipboard");
    } catch {
      toast.error("Could not copy automatically. Select the key and copy it manually.");
    }
  };

  return (
    <Dialog
      open={created !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Publishable key created</DialogTitle>
          <DialogDescription>
            Copy this key now. For security it is shown only once and cannot be recovered later.
          </DialogDescription>
        </DialogHeader>

        {created ? (
          <div className="space-y-4">
            <div className="flex items-start gap-3 rounded-lg border border-warning/25 bg-warning-soft px-3.5 py-3">
              <ShieldAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
              <p className="text-[11.5px] leading-relaxed text-warning">
                Treat this value as a secret. Do not commit it, paste it into chat, or log it.
                Storefront requests should send it as the publishable key header.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="created-key">Key</Label>
              <div className="flex gap-2">
                <Input
                  id="created-key"
                  readOnly
                  value={created.token}
                  onFocus={(event) => event.currentTarget.select()}
                  className="font-mono text-[12px]"
                />
                <Button type="button" variant="outline" className="shrink-0" onClick={copy}>
                  {copied ? (
                    <Check className="size-4" aria-hidden="true" />
                  ) : (
                    <Copy className="size-4" aria-hidden="true" />
                  )}
                  {copied ? "Copied" : "Copy"}
                </Button>
              </div>
            </div>
            <dl className="grid grid-cols-2 gap-3 text-[12px]">
              <div>
                <dt className="text-[11px] text-muted-foreground">Name</dt>
                <dd className="mt-0.5 text-foreground">{created.title}</dd>
              </div>
              <div>
                <dt className="text-[11px] text-muted-foreground">Sales channels</dt>
                <dd className="mt-0.5 font-mono text-foreground">
                  {created.sales_channel_ids.join(", ")}
                </dd>
              </div>
            </dl>
          </div>
        ) : null}

        <DialogFooter>
          <Button onClick={onClose}>
            <Globe2 className="size-4" aria-hidden="true" />I have saved the key
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
