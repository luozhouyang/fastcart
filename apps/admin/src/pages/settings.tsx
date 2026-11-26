import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  Check,
  ChevronDown,
  Copy,
  Globe,
  KeyRound,
  Loader2,
  Plus,
  ShieldAlert,
  Trash2,
  UserPlus,
  Users,
} from "lucide-react";
import { toast } from "sonner";
import type { ApiToken, CreatedApiToken, Membership, Role } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { PluginSlot } from "@/plugins/plugin-slot";
import { LocalesSection } from "@/components/locales-section";
import { EmptyState, ErrorState, PageSkeleton } from "@/components/states";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAuth } from "@/lib/auth";
import { PERMISSIONS } from "@/lib/permissions";
import { errorMessage } from "@/lib/api";
import { demoSettings } from "@/lib/demo";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { addDomain, loadSettings, saveStoreProfile, setPrimaryDomain } from "@/lib/loaders";
import type { StoreProfileView } from "@/lib/models";
import {
  SCOPE_PRESETS,
  TOKEN_TTL_OPTIONS,
  apiTokenFormError,
  createApiToken,
  isTokenActive,
  listApiTokens,
  listRoles,
  membershipRoleError,
  normalizeScopes,
  revokeApiToken,
  tokenState,
} from "@/lib/platform-team";
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
  { id: "locales", label: "Locales" },
  { id: "team", label: "Team" },
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
  const localesRef = useRef<HTMLDivElement>(null);
  const teamRef = useRef<HTMLDivElement>(null);

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
          : id === "checkout"
            ? checkoutRef.current
            : id === "locales"
              ? localesRef.current
              : teamRef.current;
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
          <Card ref={localesRef} className="scroll-mt-24 p-0">
            <LocalesSection />
          </Card>
          <Card ref={teamRef} className="scroll-mt-24 p-0">
            <TeamSection />
          </Card>
          <PluginSlot target="admin.page.settings.tabs" />
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

/* -------------------------------------------------------------------------- */
/*  Team                                                                      */
/* -------------------------------------------------------------------------- */

interface TeamData {
  memberships: Membership[];
  roles: Role[];
  tokens: ApiToken[];
}

const DEMO_ROLES: Role[] = [
  { name: "owner", description: "Full control of the store" },
  { name: "admin", description: "Administrative access to the store" },
  { name: "manager", description: "Operational access to the store" },
  { name: "product-manager", description: "Catalog and inventory access" },
  { name: "order-manager", description: "Order and customer access" },
  { name: "content-editor", description: "Content and theme access" },
  { name: "support-agent", description: "Customer support access" },
];

const DEMO_MEMBERSHIPS: Membership[] = [
  {
    id: "membership_demo_owner",
    user_id: "user_demo_zhouyang",
    store_id: "demo-store",
    status: "active",
    roles: ["owner"],
    created_at: 1_736_000_000,
    updated_at: 1_736_000_000,
  },
  {
    id: "membership_demo_ops",
    user_id: "user_demo_ops",
    store_id: "demo-store",
    status: "active",
    roles: ["order-manager", "support-agent"],
    created_at: 1_740_000_000,
    updated_at: 1_745_000_000,
  },
];

const DEMO_TOKENS: ApiToken[] = [
  {
    id: "token_demo_ci",
    user_id: "user_demo_zhouyang",
    store_id: "demo-store",
    name: "Storefront CI",
    prefix: "fc_demo123456",
    scopes: ["products.read", "inventory.read"],
    expires_at: 1_800_000_000,
    revoked_at: null,
    last_used_at: 1_763_000_000,
    created_at: 1_757_000_000,
  },
  {
    id: "token_demo_etl",
    user_id: "user_demo_zhouyang",
    store_id: "demo-store",
    name: "Analytics export",
    prefix: "fc_demo789012",
    scopes: ["orders.read", "customers.read"],
    expires_at: 1_770_000_000,
    revoked_at: 1_762_000_000,
    last_used_at: 1_761_000_000,
    created_at: 1_755_000_000,
  },
];

function teamDemoData(): TeamData {
  return {
    memberships: DEMO_MEMBERSHIPS.map((membership) => ({
      ...membership,
      roles: [...membership.roles],
    })),
    roles: DEMO_ROLES.map((role) => ({ ...role })),
    tokens: DEMO_TOKENS.map((token) => ({ ...token, scopes: [...token.scopes] })),
  };
}

const ROLE_SCOPE_CANDIDATES = [
  "products.read",
  "products.write",
  "orders.read",
  "orders.write",
  "customers.read",
  "customers.write",
  "inventory.read",
  "inventory.write",
  "promotions.read",
  "settings.read",
  "settings.write",
  "themes.read",
  "themes.write",
] as const;

function TeamSection() {
  const { mode, client, store, dataKey, can } = useAuth();
  const canWrite = can("settings.write");
  const [demoData, setDemoData] = useState<TeamData>(teamDemoData);
  const [memberDialogOpen, setMemberDialogOpen] = useState(false);
  const [memberUserId, setMemberUserId] = useState("");
  const [memberRoles, setMemberRoles] = useState<string[]>([]);
  const [memberBusy, setMemberBusy] = useState(false);
  const [memberError, setMemberError] = useState<string | null>(null);
  const [editingMembership, setEditingMembership] = useState<Membership | null>(null);
  const [removeMembershipTarget, setRemoveMembershipTarget] = useState<Membership | null>(null);
  const [removeMembershipBusy, setRemoveMembershipBusy] = useState(false);

  const [tokenDialogOpen, setTokenDialogOpen] = useState(false);
  const [tokenName, setTokenName] = useState("");
  const [tokenScopes, setTokenScopes] = useState<string[]>(["products.read"]);
  const [tokenTtl, setTokenTtl] = useState(TOKEN_TTL_OPTIONS[0]?.seconds ?? 2_592_000);
  const [tokenBusy, setTokenBusy] = useState(false);
  const [tokenError, setTokenError] = useState<string | null>(null);
  const [createdToken, setCreatedToken] = useState<CreatedApiToken | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<ApiToken | null>(null);
  const [revokeBusy, setRevokeBusy] = useState(false);

  useEffect(() => {
    setMemberDialogOpen(false);
    setEditingMembership(null);
    setRemoveMembershipTarget(null);
    setTokenDialogOpen(false);
    setCreatedToken(null);
    setRevokeTarget(null);
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<TeamData> => {
      if (mode === "demo") return Promise.resolve(demoData);
      if (!client || !store) return Promise.reject(new Error("No store is selected."));
      return Promise.all([
        client.stores.memberships.list(store.id, { signal }),
        listRoles(store.id, signal),
        listApiTokens(store.id, signal),
      ]).then(([memberships, roles, tokens]) => ({ memberships, roles, tokens }));
    },
    [mode, client, store, demoData],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoData,
  ]);

  const roles = data?.roles ?? [];
  const memberships = data?.memberships ?? [];
  const tokens = data?.tokens ?? [];

  const openAddMember = () => {
    setEditingMembership(null);
    setMemberUserId("");
    setMemberRoles([]);
    setMemberError(null);
    setMemberDialogOpen(true);
  };

  const openEditMember = (membership: Membership) => {
    setEditingMembership(membership);
    setMemberUserId(membership.user_id);
    setMemberRoles([...membership.roles]);
    setMemberError(null);
    setMemberDialogOpen(true);
  };

  const saveMembership = async () => {
    if (memberBusy) return;
    const roleMessage = membershipRoleError(memberRoles);
    if (roleMessage !== null) {
      setMemberError(roleMessage);
      return;
    }
    const userId = memberUserId.trim();
    if (userId.length === 0) {
      setMemberError("Enter the user id to add.");
      return;
    }
    setMemberBusy(true);
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        setDemoData((current) => {
          const existing = current.memberships.find((entry) => entry.user_id === userId);
          const memberships = existing
            ? current.memberships.map((entry) =>
                entry.user_id === userId
                  ? { ...entry, roles: [...memberRoles], updated_at: now }
                  : entry,
              )
            : [
                ...current.memberships,
                {
                  id: `membership_demo_${Date.now()}`,
                  user_id: userId,
                  store_id: store?.id ?? "demo-store",
                  status: "active" as const,
                  roles: [...memberRoles],
                  created_at: now,
                  updated_at: now,
                },
              ];
          return { ...current, memberships };
        });
        toast.success(editingMembership ? "Member roles updated" : "Member added");
        setMemberDialogOpen(false);
        return;
      }
      if (!client || !store) throw new Error("No store is selected.");
      await client.stores.memberships.upsert(store.id, { user_id: userId, roles: memberRoles });
      toast.success(editingMembership ? "Member roles updated" : "Member added");
      setMemberDialogOpen(false);
      reload();
    } catch (cause) {
      setMemberError(errorMessage(cause));
    } finally {
      setMemberBusy(false);
    }
  };

  const confirmRemoveMembership = async () => {
    if (!removeMembershipTarget || removeMembershipBusy) return;
    const target = removeMembershipTarget;
    setRemoveMembershipBusy(true);
    try {
      if (mode === "demo") {
        setDemoData((current) => ({
          ...current,
          memberships: current.memberships.filter((entry) => entry.user_id !== target.user_id),
        }));
      } else {
        if (!client || !store) throw new Error("No store is selected.");
        await client.stores.memberships.remove(store.id, target.user_id);
      }
      setRemoveMembershipTarget(null);
      toast.success("Member removed from the store");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRemoveMembershipBusy(false);
    }
  };

  const toggleScope = (scope: string) => {
    setTokenScopes((current) =>
      current.includes(scope) ? current.filter((entry) => entry !== scope) : [...current, scope],
    );
    setTokenError(null);
  };

  const applyScopePreset = (preset: string[]) => {
    setTokenScopes((current) => normalizeScopes([...current, ...preset]));
    setTokenError(null);
  };

  const createToken = async () => {
    if (tokenBusy) return;
    const message = apiTokenFormError({ name: tokenName, scopes: tokenScopes });
    if (message !== null) {
      setTokenError(message);
      return;
    }
    setTokenBusy(true);
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        const record: ApiToken = {
          id: `token_demo_${Date.now()}`,
          user_id: "user_demo_zhouyang",
          store_id: store?.id ?? "demo-store",
          name: tokenName.trim(),
          prefix: `fc_demo${Math.random().toString(36).slice(2, 8)}`,
          scopes: normalizeScopes(tokenScopes),
          expires_at: now + tokenTtl,
          revoked_at: null,
          last_used_at: null,
          created_at: now,
        };
        setDemoData((current) => ({ ...current, tokens: [record, ...current.tokens] }));
        setCreatedToken({
          token: `fc_demo_${Math.random().toString(36).slice(2)}`.repeat(2),
          record,
        });
      } else {
        if (!store) throw new Error("No store is selected.");
        const created = await createApiToken(store.id, {
          name: tokenName.trim(),
          store_id: store.id,
          scopes: normalizeScopes(tokenScopes),
          ttl_seconds: tokenTtl,
        });
        setCreatedToken(created);
      }
      setTokenDialogOpen(false);
      setTokenName("");
      reload();
    } catch (cause) {
      setTokenError(errorMessage(cause));
    } finally {
      setTokenBusy(false);
    }
  };

  const confirmRevoke = async () => {
    if (!revokeTarget || revokeBusy) return;
    const target = revokeTarget;
    setRevokeBusy(true);
    try {
      if (mode === "demo") {
        setDemoData((current) => ({
          ...current,
          tokens: current.tokens.map((token) =>
            token.id === target.id
              ? { ...token, revoked_at: Math.floor(Date.now() / 1000) }
              : token,
          ),
        }));
      } else {
        if (!store) throw new Error("No store is selected.");
        await revokeApiToken(store.id, target.id);
      }
      setRevokeTarget(null);
      toast.success("API token revoked");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRevokeBusy(false);
    }
  };

  if (!data && loading) {
    return (
      <div className="p-6" aria-busy="true">
        <div className="h-5 w-40 animate-pulse rounded bg-secondary" />
        <div className="mt-4 h-24 animate-pulse rounded-lg bg-secondary/60" />
      </div>
    );
  }
  if (!data && error) {
    return (
      <div className="p-6">
        <ErrorState message={error} onRetry={reload} />
      </div>
    );
  }
  if (!data) return null;

  return (
    <>
      <div className="border-b border-border px-6 py-5">
        <h2 className="text-[14px] font-medium text-foreground">Team</h2>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          Store memberships, assignable roles and API tokens for integrations
        </p>
      </div>

      <div className="border-b border-border p-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="flex items-center gap-2 text-[13px] font-medium text-foreground">
              <Users className="size-4 text-muted-foreground" aria-hidden="true" />
              Members
            </p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              A member needs an existing platform user; add them by id and pick their roles.
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={openAddMember}
            disabled={!canWrite}
            title={canWrite ? undefined : "Requires settings.write"}
          >
            <UserPlus className="size-3.5" aria-hidden="true" />
            Add member
          </Button>
        </div>

        {memberships.length === 0 ? (
          <EmptyState
            title="No members yet"
            description="Add the first teammate to let them administer this store."
            action={
              canWrite ? (
                <Button onClick={openAddMember}>
                  <UserPlus className="size-4" aria-hidden="true" />
                  Add member
                </Button>
              ) : null
            }
          />
        ) : (
          <div className="mt-4 overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">User</TableHead>
                  <TableHead className="w-[110px]">Status</TableHead>
                  <TableHead>Roles</TableHead>
                  <TableHead className="hidden w-[130px] sm:table-cell">Added</TableHead>
                  <TableHead className="w-[130px] pr-4 text-right" aria-label="Actions" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {memberships.map((membership) => (
                  <TableRow key={membership.id}>
                    <TableCell className="pl-4">
                      <p className="font-mono text-[11.5px] text-foreground">
                        {membership.user_id}
                      </p>
                      <p className="mt-0.5 font-mono text-[10px] text-faint">{membership.id}</p>
                    </TableCell>
                    <TableCell>
                      <Badge variant={membership.status === "active" ? "brand" : "secondary"}>
                        {membership.status}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1.5">
                        {membership.roles.map((role) => (
                          <Badge key={role} variant="outline">
                            {role}
                          </Badge>
                        ))}
                      </div>
                    </TableCell>
                    <TableCell className="hidden text-[11.5px] text-muted-foreground sm:table-cell">
                      {shortDate(fromUnixSeconds(membership.created_at))}
                    </TableCell>
                    <TableCell className="pr-4 text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => openEditMember(membership)}
                          disabled={!canWrite}
                        >
                          Change roles
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Remove ${membership.user_id}`}
                          onClick={() => setRemoveMembershipTarget(membership)}
                          disabled={!canWrite}
                          className="text-faint hover:text-destructive"
                        >
                          <Trash2 className="size-3.5" aria-hidden="true" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>

      <div className="border-b border-border p-6">
        <p className="flex items-center gap-2 text-[13px] font-medium text-foreground">
          <ShieldAlert className="size-4 text-muted-foreground" aria-hidden="true" />
          Roles
        </p>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          System roles bundled with Fastcart; each grants a fixed permission set.
        </p>
        <ul className="mt-3 grid gap-2 sm:grid-cols-2">
          {roles.map((role) => (
            <li key={role.name} className="rounded-lg border border-border bg-background px-3 py-2">
              <p className="font-mono text-[11.5px] text-foreground">{role.name}</p>
              <p className="mt-0.5 text-[11px] text-muted-foreground">
                {role.description.length > 0 ? role.description : "No description"}
              </p>
            </li>
          ))}
          {roles.length === 0 ? (
            <li className="text-[11.5px] text-muted-foreground">No roles returned by the API.</li>
          ) : null}
        </ul>
      </div>

      <div className="p-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <p className="flex items-center gap-2 text-[13px] font-medium text-foreground">
              <KeyRound className="size-4 text-muted-foreground" aria-hidden="true" />
              API tokens
            </p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Store-scoped tokens for scripts and integrations. The secret is shown once.
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setTokenName("");
              setTokenScopes(["products.read"]);
              setTokenError(null);
              setTokenDialogOpen(true);
            }}
            disabled={!canWrite}
            title={canWrite ? undefined : "Requires settings.write"}
          >
            <Plus className="size-3.5" aria-hidden="true" />
            New token
          </Button>
        </div>

        {tokens.length === 0 ? (
          <EmptyState
            title="No API tokens"
            description="Create a token to let an integration read or write this store."
            action={
              canWrite ? (
                <Button onClick={() => setTokenDialogOpen(true)}>
                  <Plus className="size-4" aria-hidden="true" />
                  New token
                </Button>
              ) : null
            }
          />
        ) : (
          <div className="mt-4 overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Token</TableHead>
                  <TableHead className="hidden md:table-cell">Scopes</TableHead>
                  <TableHead className="w-[100px]">State</TableHead>
                  <TableHead className="hidden w-[130px] lg:table-cell">Expires</TableHead>
                  <TableHead className="w-[90px] pr-4 text-right" aria-label="Actions" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {tokens.map((token) => {
                  const state = tokenState(token);
                  return (
                    <TableRow key={token.id}>
                      <TableCell className="pl-4">
                        <p className="text-[12px] text-foreground">{token.name}</p>
                        <p className="mt-0.5 font-mono text-[10px] text-faint">{token.prefix}…</p>
                      </TableCell>
                      <TableCell className="hidden md:table-cell">
                        <div className="flex flex-wrap gap-1">
                          {token.scopes.slice(0, 3).map((scope) => (
                            <Badge key={scope} variant="secondary">
                              {scope}
                            </Badge>
                          ))}
                          {token.scopes.length > 3 ? (
                            <Badge variant="outline">+{token.scopes.length - 3}</Badge>
                          ) : null}
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge variant={state.tone}>{state.label}</Badge>
                      </TableCell>
                      <TableCell className="hidden text-[11.5px] text-muted-foreground lg:table-cell">
                        {token.expires_at === null
                          ? "Never"
                          : shortDate(fromUnixSeconds(token.expires_at))}
                      </TableCell>
                      <TableCell className="pr-4 text-right">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setRevokeTarget(token)}
                          disabled={!canWrite || !isTokenActive(token)}
                        >
                          Revoke
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          </div>
        )}
      </div>

      <Dialog open={memberDialogOpen} onOpenChange={setMemberDialogOpen}>
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-md">
          <form
            className="space-y-5"
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              void saveMembership();
            }}
          >
            <DialogHeader>
              <DialogTitle>
                {editingMembership ? "Change member roles" : "Add a member"}
              </DialogTitle>
              <DialogDescription>
                {editingMembership
                  ? "Replaces the member's role list. At least one role is required."
                  : "The user must already exist on this server. Membership is granted by user id."}
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-2">
              <Label htmlFor="member-user-id">User id</Label>
              <Input
                id="member-user-id"
                value={memberUserId}
                onChange={(event) => {
                  setMemberUserId(event.target.value);
                  setMemberError(null);
                }}
                placeholder="user_..."
                autoComplete="off"
                spellCheck={false}
                disabled={memberBusy || editingMembership !== null}
                className="font-mono"
              />
            </div>

            <div className="space-y-2">
              <Label>Roles</Label>
              <div className="grid gap-2 sm:grid-cols-2">
                {roles.map((role) => {
                  const checked = memberRoles.includes(role.name);
                  return (
                    <button
                      key={role.name}
                      type="button"
                      aria-pressed={checked}
                      onClick={() => {
                        setMemberRoles((current) =>
                          current.includes(role.name)
                            ? current.filter((entry) => entry !== role.name)
                            : [...current, role.name],
                        );
                        setMemberError(null);
                      }}
                      disabled={memberBusy}
                      className={cn(
                        "cursor-pointer rounded-lg border px-3 py-2 text-left transition-colors",
                        checked
                          ? "border-brand/40 bg-brand-soft"
                          : "border-border bg-background hover:border-line-strong",
                      )}
                    >
                      <p className="font-mono text-[11.5px] text-foreground">{role.name}</p>
                      <p className="mt-0.5 line-clamp-2 text-[10.5px] text-muted-foreground">
                        {role.description}
                      </p>
                    </button>
                  );
                })}
              </div>
              <p className="text-[10.5px] text-faint">
                The roles endpoint lists system roles; a custom role must exist before it can be
                assigned.
              </p>
            </div>

            {memberError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {memberError}
              </p>
            ) : null}

            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                onClick={() => setMemberDialogOpen(false)}
                disabled={memberBusy}
              >
                Cancel
              </Button>
              <Button type="submit" disabled={memberBusy}>
                {memberBusy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
                {editingMembership ? "Save roles" : "Add member"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog
        open={removeMembershipTarget !== null}
        onOpenChange={(open) => {
          if (!open && !removeMembershipBusy) setRemoveMembershipTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Remove member?</DialogTitle>
            <DialogDescription>
              {removeMembershipTarget?.user_id} loses access to this store. Their platform account
              and other store memberships stay intact.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="ghost"
              onClick={() => setRemoveMembershipTarget(null)}
              disabled={removeMembershipBusy}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void confirmRemoveMembership()}
              disabled={removeMembershipBusy}
            >
              {removeMembershipBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Remove member
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={tokenDialogOpen} onOpenChange={setTokenDialogOpen}>
        <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg">
          <form
            className="space-y-5"
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              void createToken();
            }}
          >
            <DialogHeader>
              <DialogTitle>New API token</DialogTitle>
              <DialogDescription>
                Tokens are bound to your account and this store. Requested scopes must be within
                your own permissions.
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-2">
              <Label htmlFor="token-name">Name</Label>
              <Input
                id="token-name"
                value={tokenName}
                onChange={(event) => {
                  setTokenName(event.target.value);
                  setTokenError(null);
                }}
                placeholder="Storefront CI"
                autoComplete="off"
                autoFocus
                disabled={tokenBusy}
              />
            </div>

            <div className="space-y-2">
              <Label>Scopes</Label>
              <div className="flex flex-wrap gap-1.5">
                {SCOPE_PRESETS.map((preset) => (
                  <Button
                    key={preset.id}
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => applyScopePreset(preset.scopes)}
                    disabled={tokenBusy}
                  >
                    {preset.label}
                  </Button>
                ))}
              </div>
              <div className="grid max-h-48 gap-1 overflow-y-auto rounded-lg border border-border p-2 sm:grid-cols-2">
                {ROLE_SCOPE_CANDIDATES.map((scope) => {
                  const checked = tokenScopes.includes(scope);
                  return (
                    <label
                      key={scope}
                      className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-[11.5px] text-foreground hover:bg-accent"
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={() => toggleScope(scope)}
                        disabled={tokenBusy}
                        className="size-3.5 accent-[var(--brand)]"
                      />
                      <span className="font-mono">{scope}</span>
                    </label>
                  );
                })}
              </div>
            </div>

            <div className="space-y-2">
              <Label htmlFor="token-ttl">Lifetime</Label>
              <div className="relative">
                <select
                  id="token-ttl"
                  value={tokenTtl}
                  onChange={(event) => setTokenTtl(Number(event.target.value))}
                  disabled={tokenBusy}
                  className={selectClass}
                >
                  {TOKEN_TTL_OPTIONS.map((option) => (
                    <option key={option.seconds} value={option.seconds}>
                      {option.label}
                    </option>
                  ))}
                </select>
                <ChevronDown
                  className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                  aria-hidden="true"
                />
              </div>
            </div>

            {tokenError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {tokenError}
              </p>
            ) : null}

            <DialogFooter>
              <Button
                type="button"
                variant="ghost"
                onClick={() => setTokenDialogOpen(false)}
                disabled={tokenBusy}
              >
                Cancel
              </Button>
              <Button type="submit" disabled={tokenBusy}>
                {tokenBusy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
                Create token
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <TokenRevealDialog created={createdToken} onClose={() => setCreatedToken(null)} />

      <Dialog
        open={revokeTarget !== null}
        onOpenChange={(open) => {
          if (!open && !revokeBusy) setRevokeTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Revoke API token?</DialogTitle>
            <DialogDescription>
              “{revokeTarget?.name}” stops authenticating immediately. This cannot be undone.
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
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Revoke token
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function TokenRevealDialog({
  created,
  onClose,
}: {
  created: CreatedApiToken | null;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (created) setCopied(false);
  }, [created?.record.id]);

  const copy = async () => {
    if (!created) return;
    try {
      await navigator.clipboard.writeText(created.token);
      setCopied(true);
      toast.success("Token copied to the clipboard");
    } catch {
      toast.error("Could not copy automatically. Select the token and copy it manually.");
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
          <DialogTitle>API token created</DialogTitle>
          <DialogDescription>
            Copy this token now. For security it is shown only once and cannot be recovered later.
          </DialogDescription>
        </DialogHeader>

        {created ? (
          <div className="space-y-4">
            <div className="flex items-start gap-3 rounded-lg border border-warning/25 bg-warning-soft px-3.5 py-3">
              <ShieldAlert className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
              <p className="text-[11.5px] leading-relaxed text-warning">
                Treat this value as a secret. Send it as a Bearer token and never commit it or paste
                it into chat.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="created-token">Token</Label>
              <div className="flex gap-2">
                <Input
                  id="created-token"
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
                <dd className="mt-0.5 text-foreground">{created.record.name}</dd>
              </div>
              <div>
                <dt className="text-[11px] text-muted-foreground">Scopes</dt>
                <dd className="mt-0.5 font-mono text-[11px] text-foreground">
                  {created.record.scopes.join(", ") || "—"}
                </dd>
              </div>
            </dl>
          </div>
        ) : null}

        <DialogFooter>
          <Button onClick={onClose}>
            <KeyRound className="size-4" aria-hidden="true" />I have saved the token
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
