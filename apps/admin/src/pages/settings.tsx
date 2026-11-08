import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Check, ChevronDown, Globe, Loader2, Plus } from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/page-header";
import { ErrorState, PageSkeleton } from "@/components/states";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { useAuth } from "@/lib/auth";
import { PERMISSIONS } from "@/lib/permissions";
import { errorMessage } from "@/lib/api";
import { demoSettings } from "@/lib/demo";
import { addDomain, loadSettings, saveStoreProfile, setPrimaryDomain } from "@/lib/loaders";
import type { StoreProfileView } from "@/lib/models";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const CURRENCIES = [
  ["USD", "USD — US Dollar"],
  ["EUR", "EUR — Euro"],
  ["GBP", "GBP — British Pound"],
  ["CAD", "CAD — Canadian Dollar"],
  ["AUD", "AUD — Australian Dollar"],
  ["JPY", "JPY — Japanese Yen"],
] as const;

const TIMEZONES = [
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "Europe/London",
  "Europe/Berlin",
  "Europe/Paris",
  "Asia/Tokyo",
  "Asia/Shanghai",
  "Australia/Sydney",
  "UTC",
] as const;

const SECTIONS = [
  { id: "store-details", label: "Store details" },
  { id: "domains", label: "Domains" },
  { id: "checkout", label: "Checkout" },
] as const;

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-60";

function SelectField({
  id,
  label,
  value,
  onChange,
  children,
  disabled,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  children: ReactNode;
  disabled?: boolean;
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

export function SettingsPage() {
  const { mode, client, store, dataKey, refreshStores, can } = useAuth();
  const [draft, setDraft] = useState<StoreProfileView | null>(null);
  const [saving, setSaving] = useState(false);
  const [newDomain, setNewDomain] = useState("");
  const [domainBusy, setDomainBusy] = useState(false);
  const [activeSection, setActiveSection] = useState<string>("store-details");
  const detailsRef = useRef<HTMLDivElement>(null);
  const domainsRef = useRef<HTMLDivElement>(null);
  const checkoutRef = useRef<HTMLDivElement>(null);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") return Promise.resolve(demoSettings());
      if (!client || !store) return Promise.reject(new Error("No store is selected."));
      return loadSettings(client, store, signal);
    },
    [mode, client, store],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);

  useEffect(() => {
    if (data) setDraft(data);
  }, [data]);

  const scrollTo = (id: string) => {
    setActiveSection(id);
    const target =
      id === "store-details"
        ? detailsRef.current
        : id === "domains"
          ? domainsRef.current
          : checkoutRef.current;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    target?.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "start" });
  };

  const save = async () => {
    if (!draft || saving) return;
    setSaving(true);
    try {
      if (mode === "demo") {
        toast.success("Store settings saved in the demo");
      } else {
        if (!client || !store) throw new Error("No store is selected.");
        await saveStoreProfile(client, store, draft);
        await refreshStores();
        reload();
        toast.success("Store settings saved");
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  };

  const addNewDomain = async () => {
    if (!draft || domainBusy) return;
    const hostname = newDomain
      .trim()
      .toLowerCase()
      .replace(/^https?:\/\//, "")
      .replace(/\/$/, "");
    if (!/^[a-z0-9.-]+\.[a-z]{2,}$/.test(hostname)) {
      toast.error("Enter a valid hostname, for example shop.example.com.");
      return;
    }
    setDomainBusy(true);
    const primary = draft.domains.length === 0;
    try {
      if (mode === "demo") {
        setDraft({
          ...draft,
          domains: [...draft.domains, { id: `demo-domain-${Date.now()}`, hostname, primary }],
          primaryDomain: primary ? hostname : draft.primaryDomain,
        });
      } else {
        if (!client || !store) throw new Error("No store is selected.");
        await addDomain(client, draft.storeId, hostname, primary);
        await loadSettings(client, store, new AbortController().signal).then(setDraft);
      }
      setNewDomain("");
      toast.success("Domain added. Point its DNS record at this server.");
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDomainBusy(false);
    }
  };

  const makePrimary = async (hostname: string) => {
    if (!draft || domainBusy) return;
    setDomainBusy(true);
    try {
      if (mode === "demo") {
        setDraft({
          ...draft,
          primaryDomain: hostname,
          domains: draft.domains.map((domain) => ({
            ...domain,
            primary: domain.hostname === hostname,
          })),
        });
      } else {
        if (!client || !store) throw new Error("No store is selected.");
        await setPrimaryDomain(client, draft.storeId, hostname);
        await loadSettings(client, store, new AbortController().signal).then(setDraft);
      }
      toast.success(`${hostname} is now the primary domain`);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDomainBusy(false);
    }
  };

  if (!data && loading) return <PageSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data || !draft) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Settings"
        description="Store profile, domains and checkout behavior"
        actions={
          <Button
            onClick={() => void save()}
            disabled={saving || !can(PERMISSIONS.settings.write)}
            title={can(PERMISSIONS.settings.write) ? undefined : "Requires settings.write"}
          >
            {saving ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Check className="size-4" aria-hidden="true" />
            )}
            Save changes
          </Button>
        }
      />

      <div className="grid items-start gap-6 lg:grid-cols-[200px_minmax(0,1fr)]">
        <nav
          aria-label="Settings sections"
          className="hidden rounded-xl border border-border bg-card p-2 shadow-soft lg:sticky lg:top-[88px] lg:block"
        >
          {SECTIONS.map((section) => (
            <button
              key={section.id}
              type="button"
              onClick={() => scrollTo(section.id)}
              aria-current={activeSection === section.id ? "true" : undefined}
              className={cn(
                "flex h-9 w-full cursor-pointer items-center rounded-lg px-3 text-left text-[12.5px] transition-colors",
                activeSection === section.id
                  ? "bg-secondary font-medium text-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-foreground",
              )}
            >
              {section.label}
            </button>
          ))}
          <p className="px-3 pt-3 pb-1 text-[10.5px] leading-relaxed text-faint">
            Changes apply to this store only
          </p>
        </nav>

        <div className="min-w-0 space-y-4">
          <Card ref={detailsRef} className="scroll-mt-24 p-6">
            <div>
              <h2 className="text-[14px] font-medium text-foreground">Store details</h2>
              <p className="mt-1 text-[11.5px] text-muted-foreground">
                How your store appears in the admin and on receipts
              </p>
            </div>

            <div className="mt-6 grid gap-x-6 gap-y-5 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="store-name">Store name</Label>
                <Input
                  id="store-name"
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
              </div>
              <div className="space-y-2">
                <div className="flex items-baseline justify-between gap-2">
                  <Label htmlFor="store-handle">Handle</Label>
                  <span className="text-[10px] text-faint">URL-friendly</span>
                </div>
                <Input
                  id="store-handle"
                  value={draft.handle}
                  disabled
                  title="The handle is fixed when the store is provisioned"
                  className="font-mono"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="support-email">Support email</Label>
                <Input
                  id="support-email"
                  type="email"
                  value={draft.supportEmail}
                  onChange={(event) => setDraft({ ...draft, supportEmail: event.target.value })}
                  placeholder="hello@store.com"
                />
              </div>
              <SelectField
                id="store-currency"
                label="Currency"
                value={draft.currency}
                onChange={(value) => setDraft({ ...draft, currency: value })}
              >
                {CURRENCIES.map(([code, label]) => (
                  <option key={code} value={code}>
                    {label}
                  </option>
                ))}
              </SelectField>
              <SelectField
                id="store-timezone"
                label="Timezone"
                value={draft.timezone}
                onChange={(value) => setDraft({ ...draft, timezone: value })}
              >
                {TIMEZONES.map((zone) => (
                  <option key={zone} value={zone}>
                    {zone}
                  </option>
                ))}
              </SelectField>
              <SelectField
                id="store-weight"
                label="Weight unit"
                value={draft.weightUnit}
                onChange={(value) => setDraft({ ...draft, weightUnit: value })}
              >
                <option value="kg">Kilograms</option>
                <option value="g">Grams</option>
                <option value="lb">Pounds</option>
                <option value="oz">Ounces</option>
              </SelectField>
            </div>
          </Card>

          <Card ref={domainsRef} className="scroll-mt-24 p-6">
            <div>
              <h2 className="text-[14px] font-medium text-foreground">Domains</h2>
              <p className="mt-1 text-[11.5px] text-muted-foreground">
                The primary domain serves your storefront
              </p>
            </div>

            <ul className="mt-5 space-y-2">
              {draft.domains.map((domain) => (
                <li
                  key={domain.id}
                  className="flex flex-wrap items-center gap-2.5 rounded-xl border border-border bg-background px-3.5 py-3"
                >
                  <Globe className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate text-[12.5px] text-foreground">
                    {domain.hostname}
                  </span>
                  {domain.primary ? (
                    <span className="rounded-full border border-brand/20 bg-brand-soft px-2 py-0.5 text-[10px] font-medium text-brand-ink">
                      Primary
                    </span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => void makePrimary(domain.hostname)}
                      disabled={domainBusy}
                      className="cursor-pointer text-[11px] text-muted-foreground underline-offset-4 hover:text-foreground hover:underline disabled:opacity-50"
                    >
                      Make primary
                    </button>
                  )}
                </li>
              ))}
              {draft.domains.length === 0 ? (
                <li className="rounded-xl border border-dashed border-border px-3.5 py-4 text-[12px] text-muted-foreground">
                  No domains yet. Add one to serve the storefront.
                </li>
              ) : null}
            </ul>

            <div className="mt-3 flex flex-col gap-2 sm:flex-row">
              <Input
                value={newDomain}
                onChange={(event) => setNewDomain(event.target.value)}
                placeholder="Add another domain"
                aria-label="New domain"
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void addNewDomain();
                  }
                }}
                disabled={domainBusy}
              />
              <Button
                variant="outline"
                className="shrink-0"
                onClick={() => void addNewDomain()}
                disabled={domainBusy}
              >
                {domainBusy ? (
                  <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                ) : (
                  <Plus className="size-4" aria-hidden="true" />
                )}
                Add domain
              </Button>
            </div>
          </Card>

          <Card ref={checkoutRef} className="scroll-mt-24 p-6">
            <div>
              <h2 className="text-[14px] font-medium text-foreground">Checkout</h2>
              <p className="mt-1 text-[11.5px] text-muted-foreground">
                Control what customers see and provide at checkout
              </p>
            </div>

            <div className="mt-4">
              <CheckoutSwitch
                label="Guest checkout"
                description="Let customers buy without creating an account"
                checked={draft.checkout.guestCheckout}
                onChange={(checked) =>
                  setDraft({ ...draft, checkout: { ...draft.checkout, guestCheckout: checked } })
                }
              />
              <Separator />
              <CheckoutSwitch
                label="Require phone number"
                description="Ask for a phone number on the shipping step"
                checked={draft.checkout.requirePhone}
                onChange={(checked) =>
                  setDraft({ ...draft, checkout: { ...draft.checkout, requirePhone: checked } })
                }
              />
              <Separator />
              <CheckoutSwitch
                label="Order confirmation emails"
                description="Send a receipt as soon as payment is captured"
                checked={draft.checkout.confirmationEmails}
                onChange={(checked) =>
                  setDraft({
                    ...draft,
                    checkout: { ...draft.checkout, confirmationEmails: checked },
                  })
                }
              />
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}

function CheckoutSwitch({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-6 py-4">
      <div>
        <p className="text-[12.5px] font-medium text-foreground">{label}</p>
        <p className="mt-1 text-[11.5px] text-muted-foreground">{description}</p>
      </div>
      <Switch checked={checked} onCheckedChange={onChange} aria-label={label} />
    </div>
  );
}
