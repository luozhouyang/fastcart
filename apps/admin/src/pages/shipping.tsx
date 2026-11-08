import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  ChevronDown,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  Truck,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import type {
  CreateShippingOptionRequest,
  ShippingProfile,
  ShippingRate,
  UpdateShippingOptionRequest,
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
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
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
import { errorMessage, fetchAll } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { PERMISSIONS } from "@/lib/permissions";
import { consumePendingCommand, onCommand } from "@/lib/commands";
import {
  demoShippingOptions,
  demoShippingProfiles,
  demoShippingRates,
} from "@/lib/demo-operations";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  formatOptionalMinorAmount,
  normalizeCountryCode,
  normalizeProvince,
  parseOptionalMinorAmount,
  type ShippingOptionRecord,
} from "@/lib/operations";
import {
  formatMinorAmount,
  formatMinorInput,
  isValidCurrency,
  normalizeCurrency,
  parsePriceAmount,
} from "@/lib/pricing";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70";

interface ShippingData {
  profiles: ShippingProfile[];
  options: ShippingOptionRecord[];
  providers: string[];
}

interface OptionDraft {
  profileId: string;
  name: string;
  amountMinor: number;
  currency: string;
  providerId: string;
  isEnabled: boolean;
}

interface RateDraft {
  countryCode: string | null;
  provinceCode: string | null;
  amountMinor: number;
  currency: string;
  minSubtotal: number | null;
  maxSubtotal: number | null;
}

export function ShippingPage() {
  const { mode, client, store, dataKey, can } = useAuth();
  const [demoProfiles, setDemoProfiles] = useState<ShippingProfile[]>(demoShippingProfiles);
  const [demoOptions, setDemoOptions] = useState<ShippingOptionRecord[]>(demoShippingOptions);
  const [demoRates, setDemoRates] = useState<Record<string, ShippingRate[]>>(demoShippingRates);

  const [profileDialogOpen, setProfileDialogOpen] = useState(false);
  const [editingProfile, setEditingProfile] = useState<ShippingProfile | null>(null);
  const [deleteProfile, setDeleteProfile] = useState<ShippingProfile | null>(null);
  const [profileBusy, setProfileBusy] = useState(false);

  const [optionDialogOpen, setOptionDialogOpen] = useState(false);
  const [editingOption, setEditingOption] = useState<ShippingOptionRecord | null>(null);
  const [deleteOption, setDeleteOption] = useState<ShippingOptionRecord | null>(null);
  const [optionBusy, setOptionBusy] = useState(false);

  const [ratesOption, setRatesOption] = useState<ShippingOptionRecord | null>(null);
  const [rateDialogOpen, setRateDialogOpen] = useState(false);
  const [editingRate, setEditingRate] = useState<ShippingRate | null>(null);
  const [deleteRate, setDeleteRate] = useState<ShippingRate | null>(null);
  const [rateBusy, setRateBusy] = useState(false);

  const loader = useCallback(
    (signal: AbortSignal): Promise<ShippingData> => {
      if (mode === "demo") {
        return Promise.resolve({
          profiles: demoProfiles,
          options: demoOptions,
          providers: ["manual"],
        });
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return Promise.all([
        fetchAll((query) => client.shipping.profiles.list(query, { signal })),
        fetchAll((query) => client.shipping.options.list(query, { signal })),
        client.shipping.providers({ signal }).catch(() => ["manual"]),
      ]).then(([profiles, options, providers]) => ({
        profiles,
        options: options as ShippingOptionRecord[],
        providers,
      }));
    },
    [mode, client, demoProfiles, demoOptions],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    client,
    store?.id,
    dataKey,
    demoProfiles,
    demoOptions,
  ]);

  const ratesLoader = useCallback(
    (signal: AbortSignal): Promise<ShippingRate[]> => {
      const optionId = ratesOption?.id;
      if (!optionId) return Promise.resolve([]);
      if (mode === "demo") return Promise.resolve(demoRates[optionId] ?? []);
      if (!client) return Promise.reject(new Error("No store is selected."));
      return client.shipping.rates.list(optionId, { signal });
    },
    [ratesOption?.id, mode, client, demoRates],
  );

  const {
    data: rates,
    error: ratesError,
    loading: ratesLoading,
    reload: reloadRates,
  } = useAsyncData(ratesLoader, [ratesOption?.id, mode, client, store?.id, dataKey, demoRates]);

  useEffect(() => {
    setProfileDialogOpen(false);
    setEditingProfile(null);
    setDeleteProfile(null);
    setOptionDialogOpen(false);
    setEditingOption(null);
    setDeleteOption(null);
    setRatesOption(null);
    setRateDialogOpen(false);
    setEditingRate(null);
    setDeleteRate(null);
  }, [store?.id, dataKey]);

  useEffect(() => {
    const pending = consumePendingCommand();
    if (pending === "new-shipping-option") openCreateOption();
    return onCommand((command) => {
      if (command === "new-shipping-option") openCreateOption();
    });
  }, []);

  const openCreateProfile = useCallback(() => {
    setEditingProfile(null);
    setProfileDialogOpen(true);
  }, []);

  const openEditProfile = useCallback((profile: ShippingProfile) => {
    setEditingProfile(profile);
    setProfileDialogOpen(true);
  }, []);

  const openCreateOption = useCallback(() => {
    setEditingOption(null);
    setOptionDialogOpen(true);
  }, []);

  const openEditOption = useCallback((option: ShippingOptionRecord) => {
    setEditingOption(option);
    setOptionDialogOpen(true);
  }, []);

  const saveProfile = useCallback(
    async (name: string, profileType: string): Promise<void> => {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingProfile) {
          setDemoProfiles((current) =>
            current.map((profile) =>
              profile.id === editingProfile.id
                ? { ...profile, name, profile_type: profileType, updated_at: now }
                : profile,
            ),
          );
          toast.success("Shipping profile updated in the demo");
        } else {
          setDemoProfiles((current) => [
            ...current,
            {
              id: `sp_demo_${Date.now()}`,
              name,
              profile_type: profileType,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
          ]);
          toast.success("Shipping profile created in the demo");
        }
        return;
      }
      if (!client) throw new Error("No store is selected.");
      if (editingProfile) {
        await client.shipping.profiles.update(editingProfile.id, {
          name,
          profile_type: profileType,
        });
        toast.success("Shipping profile updated");
      } else {
        await client.shipping.profiles.create({ name, profile_type: profileType });
        toast.success("Shipping profile created");
      }
      reload();
    },
    [mode, client, editingProfile, reload],
  );

  const saveOption = useCallback(
    async (draft: OptionDraft): Promise<void> => {
      const body = {
        profile_id: draft.profileId,
        name: draft.name,
        amount_minor: draft.amountMinor,
        currency: draft.currency,
        provider_id: draft.providerId,
        is_enabled: draft.isEnabled,
      };
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editingOption) {
          setDemoOptions((current) =>
            current.map((option) =>
              option.id === editingOption.id
                ? {
                    ...option,
                    name: draft.name,
                    amount_minor: draft.amountMinor,
                    currency: draft.currency,
                    provider_id: draft.providerId,
                    is_enabled: draft.isEnabled,
                    updated_at: now,
                  }
                : option,
            ),
          );
          toast.success("Shipping option updated in the demo");
        } else {
          setDemoOptions((current) => [
            ...current,
            {
              id: `so_demo_${Date.now()}`,
              profile_id: draft.profileId,
              name: draft.name,
              price_type: "flat",
              amount_minor: draft.amountMinor,
              currency: draft.currency,
              provider_id: draft.providerId,
              is_enabled: draft.isEnabled,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
          ]);
          toast.success("Shipping option created in the demo");
        }
        return;
      }
      if (!client) throw new Error("No store is selected.");
      if (editingOption) {
        const update = {
          name: draft.name,
          amount_minor: draft.amountMinor,
          currency: draft.currency,
          provider_id: draft.providerId,
          is_enabled: draft.isEnabled,
        } as UpdateShippingOptionRequest;
        await client.shipping.options.update(editingOption.id, update);
        toast.success("Shipping option updated");
      } else {
        await client.shipping.options.create(body as CreateShippingOptionRequest);
        toast.success("Shipping option created");
      }
      reload();
    },
    [mode, client, editingOption, reload],
  );

  const confirmDeleteProfile = useCallback(async () => {
    if (!deleteProfile || profileBusy) return;
    const target = deleteProfile;
    setProfileBusy(true);
    try {
      if (mode === "demo") {
        setDemoProfiles((current) => current.filter((profile) => profile.id !== target.id));
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.shipping.profiles.delete(target.id);
      }
      setDeleteProfile(null);
      toast.success("Shipping profile deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setProfileBusy(false);
    }
  }, [deleteProfile, profileBusy, mode, client, reload]);

  const confirmDeleteOption = useCallback(async () => {
    if (!deleteOption || optionBusy) return;
    const target = deleteOption;
    setOptionBusy(true);
    try {
      if (mode === "demo") {
        setDemoOptions((current) => current.filter((option) => option.id !== target.id));
        setDemoRates((current) => {
          const next = { ...current };
          delete next[target.id];
          return next;
        });
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.shipping.options.delete(target.id);
      }
      if (ratesOption?.id === target.id) setRatesOption(null);
      setDeleteOption(null);
      toast.success("Shipping option deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setOptionBusy(false);
    }
  }, [deleteOption, optionBusy, mode, client, reload, ratesOption?.id]);

  const saveRate = useCallback(
    async (draft: RateDraft): Promise<void> => {
      const option = ratesOption;
      if (!option) throw new Error("Select a shipping option first.");
      const body = {
        country_code: draft.countryCode,
        province_code: draft.provinceCode,
        amount_minor: draft.amountMinor,
        currency: draft.currency,
        min_subtotal: draft.minSubtotal,
        max_subtotal: draft.maxSubtotal,
      };
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        const current = demoRates[option.id] ?? [];
        if (editingRate) {
          setDemoRates((previous) => ({
            ...previous,
            [option.id]: current.map((rate) =>
              rate.id === editingRate.id ? { ...rate, ...body } : rate,
            ),
          }));
          toast.success("Shipping rate updated in the demo");
        } else {
          setDemoRates((previous) => ({
            ...previous,
            [option.id]: [
              ...current,
              {
                id: `sr_demo_${Date.now()}`,
                shipping_option_id: option.id,
                ...body,
                metadata: {},
                created_at: now,
              },
            ],
          }));
          toast.success("Shipping rate created in the demo");
        }
        return;
      }
      if (!client) throw new Error("No store is selected.");
      if (editingRate) {
        await client.shipping.rates.update(editingRate.id, body);
        toast.success("Shipping rate updated");
      } else {
        await client.shipping.rates.create(option.id, body);
        toast.success("Shipping rate created");
      }
      reloadRates();
    },
    [ratesOption, mode, client, editingRate, demoRates, reloadRates],
  );

  const confirmDeleteRate = useCallback(async () => {
    if (!deleteRate || rateBusy) return;
    const target = deleteRate;
    const option = ratesOption;
    setRateBusy(true);
    try {
      if (mode === "demo") {
        if (!option) return;
        setDemoRates((current) => ({
          ...current,
          [option.id]: (current[option.id] ?? []).filter((rate) => rate.id !== target.id),
        }));
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.shipping.rates.delete(target.id);
      }
      setDeleteRate(null);
      toast.success("Shipping rate deleted");
      reloadRates();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRateBusy(false);
    }
  }, [deleteRate, rateBusy, ratesOption, mode, client, reloadRates]);

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const { profiles, options, providers } = data;
  const profileName = (profileId: string) =>
    profiles.find((profile) => profile.id === profileId)?.name ?? profileId;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Shipping"
        description="Define shipping profiles, flat-rate options and the destination rates that override them."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              disabled={loading}
              aria-label="Refresh shipping"
              title="Refresh shipping"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            {can(PERMISSIONS.settings.write) ? (
              <Button onClick={openCreateOption} disabled={profiles.length === 0}>
                <Plus className="size-4" aria-hidden="true" />
                New option
              </Button>
            ) : null}
          </>
        }
      />

      {profiles.length === 0 ? (
        <Card className="px-6 py-4 text-[12px] text-muted-foreground">
          Create a shipping profile before adding options.
        </Card>
      ) : null}

      <Card className="overflow-hidden">
        <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Shipping profiles</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Products are assigned to a profile so their rates stay independent.
            </p>
          </div>
          {can(PERMISSIONS.settings.write) ? (
            <Button variant="outline" size="sm" onClick={openCreateProfile}>
              <Plus className="size-3.5" aria-hidden="true" />
              New profile
            </Button>
          ) : null}
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Profile</TableHead>
              <TableHead className="w-[120px]">Type</TableHead>
              <TableHead className="w-[110px]">Options</TableHead>
              <TableHead className="hidden w-[120px] md:table-cell">Created</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {profiles.map((profile) => (
              <TableRow key={profile.id}>
                <TableCell className="pl-6">
                  <span className="text-[13px] font-medium text-foreground">{profile.name}</span>
                  <p className="mt-1 font-mono text-[10.5px] text-faint">{profile.id}</p>
                </TableCell>
                <TableCell>
                  <Badge variant="secondary">{profile.profile_type}</Badge>
                </TableCell>
                <TableCell className="text-[12px] text-ink-soft tnum">
                  {options.filter((option) => option.profile_id === profile.id).length}
                </TableCell>
                <TableCell className="hidden text-muted-foreground md:table-cell">
                  {shortDate(fromUnixSeconds(profile.created_at))}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Actions for ${profile.name}`}
                      >
                        <MoreHorizontal className="size-4" aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-[190px]">
                      <DropdownMenuLabel className="truncate">{profile.name}</DropdownMenuLabel>
                      <DropdownMenuItem onSelect={() => openEditProfile(profile)}>
                        <Pencil aria-hidden="true" />
                        Edit profile
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        variant="destructive"
                        onSelect={() => setDeleteProfile(profile)}
                      >
                        <Trash2 aria-hidden="true" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
            {profiles.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={5} className="p-0">
                  <EmptyState
                    title="No shipping profiles"
                    description="Create a profile to group products that share shipping rates."
                    action={
                      <Button onClick={openCreateProfile}>
                        <Plus className="size-4" aria-hidden="true" />
                        New profile
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
            <p className="text-[13.5px] font-medium text-foreground">Shipping options</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Flat amounts are charged unless a destination rate matches the cart.
            </p>
          </div>
          <p className="text-[11.5px] text-muted-foreground">
            {options.length} {options.length === 1 ? "option" : "options"}
            {mode === "demo" ? " · sample data" : ""}
          </p>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Option</TableHead>
              <TableHead className="hidden w-[150px] md:table-cell">Profile</TableHead>
              <TableHead className="w-[120px]">Flat rate</TableHead>
              <TableHead className="hidden w-[110px] lg:table-cell">Provider</TableHead>
              <TableHead className="w-[100px]">Status</TableHead>
              <TableHead className="hidden w-[120px] xl:table-cell">Updated</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {options.map((option) => (
              <TableRow key={option.id}>
                <TableCell className="pl-6">
                  <button
                    type="button"
                    onClick={() => setRatesOption(option)}
                    className="cursor-pointer text-left text-[13px] font-medium text-foreground underline-offset-4 hover:underline"
                  >
                    {option.name}
                  </button>
                  <p className="mt-1 font-mono text-[10.5px] text-faint">{option.id}</p>
                </TableCell>
                <TableCell className="hidden text-[12px] text-ink-soft md:table-cell">
                  {profileName(option.profile_id)}
                </TableCell>
                <TableCell className="font-medium text-foreground tnum">
                  {formatMinorAmount(option.amount_minor, option.currency)}
                </TableCell>
                <TableCell className="hidden lg:table-cell">
                  <Badge variant="outline">{option.provider_id ?? "manual"}</Badge>
                </TableCell>
                <TableCell>
                  {option.is_enabled ? (
                    <Badge variant="brand">Enabled</Badge>
                  ) : (
                    <Badge variant="secondary">Disabled</Badge>
                  )}
                </TableCell>
                <TableCell className="hidden text-muted-foreground xl:table-cell">
                  {shortDate(fromUnixSeconds(option.updated_at))}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Actions for ${option.name}`}
                      >
                        <MoreHorizontal className="size-4" aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-[190px]">
                      <DropdownMenuLabel className="truncate">{option.name}</DropdownMenuLabel>
                      <DropdownMenuItem onSelect={() => setRatesOption(option)}>
                        <Truck aria-hidden="true" />
                        Manage rates
                      </DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => openEditOption(option)}>
                        <Pencil aria-hidden="true" />
                        Edit option
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        variant="destructive"
                        onSelect={() => setDeleteOption(option)}
                      >
                        <Trash2 aria-hidden="true" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
            {options.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={7} className="p-0">
                  <EmptyState
                    title="No shipping options"
                    description="Add a flat-rate option so customers can choose how their order ships."
                    action={
                      <Button onClick={openCreateOption} disabled={profiles.length === 0}>
                        <Plus className="size-4" aria-hidden="true" />
                        New option
                      </Button>
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </Card>

      <ProfileDialog
        open={profileDialogOpen}
        onOpenChange={(open) => {
          setProfileDialogOpen(open);
          if (!open) setEditingProfile(null);
        }}
        profile={editingProfile}
        onSubmit={saveProfile}
      />

      <OptionDialog
        open={optionDialogOpen}
        onOpenChange={(open) => {
          setOptionDialogOpen(open);
          if (!open) setEditingOption(null);
        }}
        option={editingOption}
        profiles={profiles}
        providers={providers}
        defaultCurrency={normalizeCurrency(store?.currency_code, "USD")}
        onSubmit={saveOption}
      />

      <Sheet
        open={ratesOption !== null}
        onOpenChange={(open) => {
          if (!open) setRatesOption(null);
        }}
      >
        <SheetContent className="flex flex-col sm:max-w-2xl">
          {ratesOption ? (
            <>
              <SheetHeader>
                <SheetTitle>{ratesOption.name} rates</SheetTitle>
                <SheetDescription>
                  Destination overrides for the{" "}
                  {formatMinorAmount(ratesOption.amount_minor, ratesOption.currency)} flat amount.
                  The most specific matching rate wins.
                </SheetDescription>
              </SheetHeader>

              <div className="flex-1 overflow-y-auto p-6">
                {ratesError ? (
                  <div className="rounded-xl border border-border px-4 py-6 text-center">
                    <p className="text-[12.5px] text-destructive">{ratesError}</p>
                    <Button variant="outline" size="sm" className="mt-3" onClick={reloadRates}>
                      <RefreshCw className="size-3.5" aria-hidden="true" />
                      Try again
                    </Button>
                  </div>
                ) : ratesLoading ? (
                  <p className="text-[12.5px] text-muted-foreground">Loading rates…</p>
                ) : (rates ?? []).length === 0 ? (
                  <EmptyState
                    title="No destination rates"
                    description="The flat amount applies everywhere. Add a rate to charge a different amount for a country, province or subtotal range."
                    action={
                      can(PERMISSIONS.settings.write) ? (
                        <Button
                          variant="outline"
                          onClick={() => {
                            setEditingRate(null);
                            setRateDialogOpen(true);
                          }}
                        >
                          <Plus className="size-4" aria-hidden="true" />
                          Add rate
                        </Button>
                      ) : null
                    }
                  />
                ) : (
                  <Table>
                    <TableHeader>
                      <TableRow className="hover:bg-transparent">
                        <TableHead>Destination</TableHead>
                        <TableHead className="w-[150px]">Subtotal</TableHead>
                        <TableHead className="w-[110px]">Amount</TableHead>
                        <TableHead className="w-10" aria-label="Actions" />
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {(rates ?? []).map((rate) => (
                        <TableRow key={rate.id}>
                          <TableCell className="text-[12.5px] text-foreground">
                            {rate.country_code === null
                              ? "All countries"
                              : rate.province_code === null
                                ? rate.country_code
                                : `${rate.country_code} · ${rate.province_code}`}
                          </TableCell>
                          <TableCell className="text-[12px] text-muted-foreground">
                            {rate.min_subtotal === null && rate.max_subtotal === null
                              ? "Any"
                              : `${rate.min_subtotal === null ? "—" : formatMinorAmount(rate.min_subtotal, rate.currency)} – ${
                                  rate.max_subtotal === null
                                    ? "—"
                                    : formatMinorAmount(rate.max_subtotal, rate.currency)
                                }`}
                          </TableCell>
                          <TableCell className="font-medium text-foreground tnum">
                            {formatMinorAmount(rate.amount_minor, rate.currency)}
                          </TableCell>
                          <TableCell className="text-right">
                            <DropdownMenu>
                              <DropdownMenuTrigger asChild>
                                <Button variant="ghost" size="icon-sm" aria-label="Rate actions">
                                  <MoreHorizontal className="size-4" aria-hidden="true" />
                                </Button>
                              </DropdownMenuTrigger>
                              <DropdownMenuContent align="end" className="w-[170px]">
                                <DropdownMenuItem
                                  onSelect={() => {
                                    setEditingRate(rate);
                                    setRateDialogOpen(true);
                                  }}
                                >
                                  <Pencil aria-hidden="true" />
                                  Edit rate
                                </DropdownMenuItem>
                                <DropdownMenuItem
                                  variant="destructive"
                                  onSelect={() => setDeleteRate(rate)}
                                >
                                  <Trash2 aria-hidden="true" />
                                  Delete
                                </DropdownMenuItem>
                              </DropdownMenuContent>
                            </DropdownMenu>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                )}
              </div>

              <SheetFooter>
                {can(PERMISSIONS.settings.write) ? (
                  <Button
                    variant="outline"
                    onClick={() => {
                      setEditingRate(null);
                      setRateDialogOpen(true);
                    }}
                  >
                    <Plus className="size-4" aria-hidden="true" />
                    Add rate
                  </Button>
                ) : null}
                <Button variant="ghost" onClick={() => setRatesOption(null)}>
                  Close
                </Button>
              </SheetFooter>
            </>
          ) : null}
        </SheetContent>
      </Sheet>

      <RateDialog
        open={rateDialogOpen}
        onOpenChange={(open) => {
          setRateDialogOpen(open);
          if (!open) setEditingRate(null);
        }}
        rate={editingRate}
        option={ratesOption}
        onSubmit={saveRate}
      />

      <ConfirmDialog
        open={deleteProfile !== null}
        title="Delete shipping profile?"
        description={`“${deleteProfile?.name}” will be removed. Options assigned to it are not deleted automatically; reassign or remove them first.`}
        busy={profileBusy}
        onCancel={() => setDeleteProfile(null)}
        onConfirm={() => void confirmDeleteProfile()}
      />

      <ConfirmDialog
        open={deleteOption !== null}
        title="Delete shipping option?"
        description={`“${deleteOption?.name}” and its destination rates will stop being offered at checkout.`}
        busy={optionBusy}
        onCancel={() => setDeleteOption(null)}
        onConfirm={() => void confirmDeleteOption()}
      />

      <ConfirmDialog
        open={deleteRate !== null}
        title="Delete shipping rate?"
        description="The option's flat amount will apply to this destination again."
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

function ProfileDialog({
  open,
  onOpenChange,
  profile,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  profile: ShippingProfile | null;
  onSubmit: (name: string, profileType: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [profileType, setProfileType] = useState("default");
  const [nameError, setNameError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setName(profile?.name ?? "");
    setProfileType(profile?.profile_type ?? "default");
    setNameError(null);
    setBusy(false);
  }, [open, profile?.id]);

  const submit = async () => {
    if (busy) return;
    const trimmed = name.trim();
    if (trimmed.length === 0) {
      setNameError("Enter a name for this profile.");
      return;
    }
    setBusy(true);
    try {
      await onSubmit(trimmed, profileType.trim().length === 0 ? "default" : profileType.trim());
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
            <DialogTitle>{profile ? "Edit shipping profile" : "New shipping profile"}</DialogTitle>
            <DialogDescription>
              Profiles group products that share shipping options and rates.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="profile-name">Name</Label>
            <Input
              id="profile-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setNameError(null);
              }}
              placeholder="Default"
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
            <Label htmlFor="profile-type">Type</Label>
            <Input
              id="profile-type"
              value={profileType}
              onChange={(event) => setProfileType(event.target.value)}
              placeholder="default"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              className="font-mono"
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
              {profile ? "Save changes" : "Create profile"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function OptionDialog({
  open,
  onOpenChange,
  option,
  profiles,
  providers,
  defaultCurrency,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  option: ShippingOptionRecord | null;
  profiles: ShippingProfile[];
  providers: string[];
  defaultCurrency: string;
  onSubmit: (draft: OptionDraft) => Promise<void>;
}) {
  const [profileId, setProfileId] = useState("");
  const [name, setName] = useState("");
  const [amount, setAmount] = useState("");
  const [currency, setCurrency] = useState(defaultCurrency);
  const [providerId, setProviderId] = useState("manual");
  const [isEnabled, setIsEnabled] = useState(true);
  const [nameError, setNameError] = useState<string | null>(null);
  const [amountError, setAmountError] = useState<string | null>(null);
  const [currencyError, setCurrencyError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setProfileId(option?.profile_id ?? profiles[0]?.id ?? "");
    setName(option?.name ?? "");
    setAmount(option ? formatMinorInput(option.amount_minor) : "");
    setCurrency(option?.currency ?? defaultCurrency);
    setProviderId(option?.provider_id ?? providers[0] ?? "manual");
    setIsEnabled(option?.is_enabled ?? true);
    setNameError(null);
    setAmountError(null);
    setCurrencyError(null);
    setBusy(false);
  }, [open, option?.id, profiles, providers, defaultCurrency]);

  const providerOptions =
    providerId.length > 0 && !providers.includes(providerId)
      ? [providerId, ...providers]
      : providers;

  const submit = async () => {
    if (busy) return;
    const trimmed = name.trim();
    if (trimmed.length === 0) {
      setNameError("Enter a name for this option.");
      return;
    }
    if (profileId.length === 0) {
      setNameError("Choose a shipping profile.");
      return;
    }
    if (!isValidCurrency(currency)) {
      setCurrencyError("Enter a three-letter ISO currency code.");
      return;
    }
    const parsed = parsePriceAmount(amount);
    if (!parsed.ok) {
      setAmountError(parsed.message);
      return;
    }
    setBusy(true);
    try {
      await onSubmit({
        profileId,
        name: trimmed,
        amountMinor: parsed.minor,
        currency: normalizeCurrency(currency),
        providerId: providerId.trim().length === 0 ? "manual" : providerId.trim(),
        isEnabled,
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
            <DialogTitle>{option ? "Edit shipping option" : "New shipping option"}</DialogTitle>
            <DialogDescription>
              {option
                ? "Update the flat amount, provider or availability of this option."
                : "Create a flat-rate option in a shipping profile."}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="option-name">Name</Label>
            <Input
              id="option-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setNameError(null);
              }}
              placeholder="Standard shipping"
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
            <SelectField
              id="option-profile"
              label="Profile"
              value={profileId}
              onChange={setProfileId}
              disabled={busy || option !== null}
            >
              {profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.name}
                </option>
              ))}
            </SelectField>
            <SelectField
              id="option-provider"
              label="Provider"
              value={providerId}
              onChange={setProviderId}
              disabled={busy}
            >
              {providerOptions.map((provider) => (
                <option key={provider} value={provider}>
                  {provider}
                </option>
              ))}
            </SelectField>
          </div>

          <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_120px]">
            <div className="space-y-2">
              <Label htmlFor="option-amount">Flat amount</Label>
              <Input
                id="option-amount"
                type="number"
                min="0"
                step="0.01"
                inputMode="decimal"
                value={amount}
                onChange={(event) => {
                  setAmount(event.target.value);
                  setAmountError(null);
                }}
                placeholder="5.00"
                disabled={busy}
                aria-invalid={amountError !== null}
              />
              {amountError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {amountError}
                </p>
              ) : null}
            </div>
            <div className="space-y-2">
              <Label htmlFor="option-currency">Currency</Label>
              <Input
                id="option-currency"
                value={currency}
                onChange={(event) => {
                  setCurrency(event.target.value.toUpperCase().slice(0, 3));
                  setCurrencyError(null);
                }}
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

          <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3.5">
            <div>
              <Label htmlFor="option-enabled">Enabled</Label>
              <p className="mt-1 text-[11.5px] text-muted-foreground">
                Disabled options are hidden at checkout.
              </p>
            </div>
            <Switch
              id="option-enabled"
              checked={isEnabled}
              onCheckedChange={setIsEnabled}
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
              {option ? "Save changes" : "Create option"}
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
  option,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  rate: ShippingRate | null;
  option: ShippingOptionRecord | null;
  onSubmit: (draft: RateDraft) => Promise<void>;
}) {
  const [country, setCountry] = useState("");
  const [province, setProvince] = useState("");
  const [amount, setAmount] = useState("");
  const [currency, setCurrency] = useState("USD");
  const [minSubtotal, setMinSubtotal] = useState("");
  const [maxSubtotal, setMaxSubtotal] = useState("");
  const [countryError, setCountryError] = useState<string | null>(null);
  const [amountError, setAmountError] = useState<string | null>(null);
  const [subtotalError, setSubtotalError] = useState<string | null>(null);
  const [currencyError, setCurrencyError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setCountry(rate?.country_code ?? "");
    setProvince(rate?.province_code ?? "");
    setAmount(rate ? formatMinorInput(rate.amount_minor) : "");
    setCurrency(rate?.currency ?? option?.currency ?? "USD");
    setMinSubtotal(formatOptionalMinorAmount(rate?.min_subtotal ?? null));
    setMaxSubtotal(formatOptionalMinorAmount(rate?.max_subtotal ?? null));
    setCountryError(null);
    setAmountError(null);
    setSubtotalError(null);
    setCurrencyError(null);
    setBusy(false);
  }, [open, rate?.id, option?.id, option?.currency]);

  const submit = async () => {
    if (busy) return;
    const countryCode = country.trim().length === 0 ? null : normalizeCountryCode(country);
    if (country.trim().length > 0 && countryCode === null) {
      setCountryError("Enter a two-letter ISO country code, or leave it empty for all countries.");
      return;
    }
    if (!isValidCurrency(currency)) {
      setCurrencyError("Enter a three-letter ISO currency code.");
      return;
    }
    const parsedAmount = parsePriceAmount(amount);
    if (!parsedAmount.ok) {
      setAmountError(parsedAmount.message);
      return;
    }
    const min = parseOptionalMinorAmount(minSubtotal);
    const max = parseOptionalMinorAmount(maxSubtotal);
    if (!min.ok) {
      setSubtotalError(min.message);
      return;
    }
    if (!max.ok) {
      setSubtotalError(max.message);
      return;
    }
    if (min.minor !== null && max.minor !== null && min.minor > max.minor) {
      setSubtotalError("The minimum subtotal must be less than or equal to the maximum.");
      return;
    }

    setBusy(true);
    try {
      await onSubmit({
        countryCode,
        provinceCode: normalizeProvince(province),
        amountMinor: parsedAmount.minor,
        currency: normalizeCurrency(currency),
        minSubtotal: min.minor,
        maxSubtotal: max.minor,
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
            <DialogTitle>{rate ? "Edit shipping rate" : "New shipping rate"}</DialogTitle>
            <DialogDescription>
              {option
                ? `Override the ${formatMinorAmount(option.amount_minor, option.currency)} flat amount for ${option.name}.`
                : "Add a destination-specific rate."}
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="rate-country">Country</Label>
              <Input
                id="rate-country"
                value={country}
                onChange={(event) => {
                  setCountry(event.target.value.toUpperCase().slice(0, 2));
                  setCountryError(null);
                }}
                placeholder="All countries"
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
              <Label htmlFor="rate-province">Province / state</Label>
              <Input
                id="rate-province"
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

          <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_120px]">
            <div className="space-y-2">
              <Label htmlFor="rate-amount">Amount</Label>
              <Input
                id="rate-amount"
                type="number"
                min="0"
                step="0.01"
                inputMode="decimal"
                value={amount}
                onChange={(event) => {
                  setAmount(event.target.value);
                  setAmountError(null);
                }}
                placeholder="9.00"
                disabled={busy}
                aria-invalid={amountError !== null}
              />
              {amountError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {amountError}
                </p>
              ) : null}
            </div>
            <div className="space-y-2">
              <Label htmlFor="rate-currency">Currency</Label>
              <Input
                id="rate-currency"
                value={currency}
                onChange={(event) => {
                  setCurrency(event.target.value.toUpperCase().slice(0, 3));
                  setCurrencyError(null);
                }}
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

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="rate-min">Minimum subtotal</Label>
              <Input
                id="rate-min"
                type="number"
                min="0"
                step="0.01"
                inputMode="decimal"
                value={minSubtotal}
                onChange={(event) => {
                  setMinSubtotal(event.target.value);
                  setSubtotalError(null);
                }}
                placeholder="No minimum"
                disabled={busy}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="rate-max">Maximum subtotal</Label>
              <Input
                id="rate-max"
                type="number"
                min="0"
                step="0.01"
                inputMode="decimal"
                value={maxSubtotal}
                onChange={(event) => {
                  setMaxSubtotal(event.target.value);
                  setSubtotalError(null);
                }}
                placeholder="No maximum"
                disabled={busy}
              />
            </div>
          </div>
          <p className="-mt-2 text-[11px] leading-relaxed text-faint">
            Leave country empty to match every destination. Subtotal bounds are optional and use the
            cart subtotal in the rate currency.
          </p>
          {subtotalError ? (
            <p className="-mt-2 text-[11.5px] text-destructive" role="alert">
              {subtotalError}
            </p>
          ) : null}

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
