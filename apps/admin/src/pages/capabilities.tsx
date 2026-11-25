import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Blocks, ChevronDown, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/page-header";
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
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import {
  bindCapability,
  contributionForKey,
  groupCapabilities,
  initialConfig,
  initialContribution,
  isBoundProvider,
  listCapabilities,
  parseConfigObject,
  providerKey,
  unbindCapability,
  type CapabilitiesResponse,
  type CapabilityContribution,
  type CapabilityGroup,
  type JsonObject,
} from "@/lib/capabilities";
import { fromUnixSeconds, shortDate, timeOfDay } from "@/lib/format";
import { useAsyncData } from "@/lib/use-async-data";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-60";

function formatBindingTime(timestamp: number): string {
  const iso = fromUnixSeconds(timestamp);
  return `${shortDate(iso)} at ${timeOfDay(iso)}`;
}

function bindingProviderLabel(group: CapabilityGroup): string {
  const contribution = group.binding
    ? group.contributions.find((entry) => isBoundProvider(group.binding, entry))
    : undefined;
  if (contribution) return `${contribution.plugin_name} v${contribution.plugin_version}`;
  return group.binding ? `${group.binding.plugin_id} v${group.binding.version}` : "";
}

function CapabilityCard({
  group,
  canWrite,
  onEdit,
  onUnbind,
}: {
  group: CapabilityGroup;
  canWrite: boolean;
  onEdit: () => void;
  onUnbind: () => void;
}) {
  const { binding } = group;
  const boundContribution = binding
    ? (group.contributions.find((contribution) => isBoundProvider(binding, contribution)) ?? null)
    : null;

  return (
    <Card className="p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-brand-ink">
            <Blocks className="size-4" aria-hidden="true" />
          </span>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="text-[13px] font-medium text-foreground">{group.title}</h2>
              <Badge variant={binding ? (binding.enabled ? "brand" : "secondary") : "outline"}>
                {binding ? (binding.enabled ? "Enabled" : "Disabled") : "Not bound"}
              </Badge>
            </div>
            <p className="mt-1 font-mono text-[10.5px] text-faint">{group.id}</p>
          </div>
        </div>
      </div>

      <div className="mt-4">
        <p className="text-[10.5px] font-medium tracking-[0.08em] text-faint uppercase">
          Providers
        </p>
        {group.contributions.length === 0 ? (
          <p className="mt-1.5 rounded-lg border border-warning/25 bg-warning-soft px-3 py-2 text-[11.5px] leading-relaxed text-warning">
            No enabled plugin currently provides this capability. Enable a provider or unbind the
            stored binding.
          </p>
        ) : (
          <ul className="mt-1.5 divide-y divide-border overflow-hidden rounded-lg border border-border">
            {group.contributions.map((contribution) => {
              const active = isBoundProvider(binding, contribution);
              return (
                <li
                  key={providerKey(contribution)}
                  className="flex flex-wrap items-center justify-between gap-2 bg-background px-3 py-2"
                >
                  <span className="min-w-0">
                    <span className="block text-[12px] text-foreground">
                      {contribution.plugin_name}
                    </span>
                    <span className="mt-0.5 block font-mono text-[10px] text-faint">
                      {contribution.plugin_id} · plugin v{contribution.plugin_version} · capability
                      v{contribution.version}
                    </span>
                  </span>
                  {active ? <Badge variant="brand">Bound</Badge> : null}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {binding ? (
        <div className="mt-4 space-y-2">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-muted-foreground">
            <span>Bound to</span>
            <span className="font-medium text-foreground">{bindingProviderLabel(group)}</span>
            <span className="font-mono text-[10.5px] text-faint">
              capability v{binding.version} · updated {formatBindingTime(binding.updated_at)}
            </span>
          </div>
          <pre className="max-h-48 overflow-auto rounded-lg border border-border bg-secondary/40 p-3 font-mono text-[10.5px] leading-relaxed text-ink-soft">
            {JSON.stringify(binding.config, null, 2)}
          </pre>
          {!boundContribution ? (
            <p className="rounded-lg border border-warning/25 bg-warning-soft px-3 py-2 text-[11px] leading-relaxed text-warning">
              This binding points to a provider that is not currently enabled or no longer offers
              this version. Rebind it or unbind it.
            </p>
          ) : null}
        </div>
      ) : (
        <p className="mt-4 text-[11.5px] leading-relaxed text-muted-foreground">
          No provider is bound to this capability. The storefront cannot use it until an admin binds
          one.
        </p>
      )}

      <div className="mt-4 flex flex-wrap items-center gap-2">
        {canWrite && group.contributions.length > 0 ? (
          <Button variant="outline" size="sm" onClick={onEdit}>
            {binding ? (
              <Pencil className="size-3.5" aria-hidden="true" />
            ) : (
              <Plus className="size-3.5" aria-hidden="true" />
            )}
            {binding ? "Edit binding" : "Bind capability"}
          </Button>
        ) : null}
        {canWrite && binding ? (
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive hover:text-destructive"
            onClick={onUnbind}
          >
            <Trash2 className="size-3.5" aria-hidden="true" />
            Unbind
          </Button>
        ) : null}
        {!canWrite ? (
          <span className="text-[10.5px] text-faint">
            Changing a binding requires the plugins.write permission.
          </span>
        ) : null}
      </div>
    </Card>
  );
}

function CapabilityDialog({
  group,
  busy,
  onOpenChange,
  onSubmit,
}: {
  group: CapabilityGroup | null;
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    group: CapabilityGroup,
    contribution: CapabilityContribution,
    config: JsonObject,
    enabled: boolean,
  ) => void;
}) {
  const [provider, setProvider] = useState("");
  const [configText, setConfigText] = useState("{}");
  const [enabled, setEnabled] = useState(true);
  const [configError, setConfigError] = useState<string | null>(null);

  useEffect(() => {
    if (!group) return;
    const contribution = initialContribution(group);
    setProvider(contribution ? providerKey(contribution) : "");
    setConfigText(JSON.stringify(initialConfig(group, contribution), null, 2));
    setEnabled(group.binding?.enabled ?? true);
    setConfigError(null);
  }, [group]);

  const selected = contributionForKey(group, provider);
  const editing = group !== null && group.binding !== null;
  const multipleProviders = (group?.contributions.length ?? 0) > 1;

  const chooseProvider = (key: string) => {
    setProvider(key);
    if (!group) return;
    setConfigText(JSON.stringify(initialConfig(group, contributionForKey(group, key)), null, 2));
    setConfigError(null);
  };

  const submit = () => {
    if (!group || !selected) return;
    const parsed = parseConfigObject(configText);
    if (parsed.error !== null || parsed.config === null) {
      setConfigError(parsed.error ?? "Config must be a JSON object.");
      return;
    }
    setConfigError(null);
    onSubmit(group, selected, parsed.config, enabled);
  };

  return (
    <Dialog open={group !== null} onOpenChange={(open) => !open && onOpenChange(false)}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{editing ? "Edit binding" : "Bind capability"}</DialogTitle>
          <DialogDescription>
            {group
              ? `Choose the plugin that provides "${group.title}" for this store and configure it.`
              : ""}
          </DialogDescription>
        </DialogHeader>

        {group ? (
          <div className="space-y-4">
            <div className="rounded-lg border border-border bg-background px-3.5 py-3">
              <p className="text-[10.5px] font-medium tracking-[0.08em] text-faint uppercase">
                Capability
              </p>
              <p className="mt-1 text-[12.5px] text-foreground">{group.title}</p>
              <p className="mt-0.5 font-mono text-[10.5px] text-faint">{group.id}</p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="capability-provider">Provider</Label>
              {multipleProviders ? (
                <div className="relative">
                  <select
                    id="capability-provider"
                    value={provider}
                    onChange={(event) => chooseProvider(event.target.value)}
                    disabled={busy}
                    className={selectClass}
                  >
                    {group.contributions.map((contribution) => (
                      <option key={providerKey(contribution)} value={providerKey(contribution)}>
                        {contribution.plugin_name} · plugin v{contribution.plugin_version} ·
                        capability v{contribution.version}
                      </option>
                    ))}
                  </select>
                  <ChevronDown
                    className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                    aria-hidden="true"
                  />
                </div>
              ) : selected ? (
                <p className="rounded-lg border border-border bg-background px-3 py-2.5 text-[12.5px] text-foreground">
                  {selected.plugin_name}{" "}
                  <span className="font-mono text-[10.5px] text-faint">
                    plugin v{selected.plugin_version} · capability v{selected.version}
                  </span>
                </p>
              ) : null}
            </div>

            <div className="space-y-2">
              <Label htmlFor="capability-config">Configuration (JSON)</Label>
              <textarea
                id="capability-config"
                value={configText}
                onChange={(event) => {
                  setConfigText(event.target.value);
                  setConfigError(null);
                }}
                spellCheck={false}
                rows={9}
                disabled={busy}
                aria-invalid={configError !== null}
                {...(configError ? { "aria-describedby": "capability-config-error" } : {})}
                className="w-full rounded-lg border border-border bg-background p-3 font-mono text-[11.5px] text-foreground outline-none hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-destructive/20"
              />
              {configError ? (
                <p
                  id="capability-config-error"
                  role="alert"
                  className="text-[11px] text-destructive"
                >
                  {configError}
                </p>
              ) : (
                <p className="text-[10.5px] text-faint">
                  The provider validates this object against its config schema when the binding is
                  saved.
                </p>
              )}
            </div>

            <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-background px-3.5 py-3">
              <div>
                <p className="text-[12px] font-medium text-foreground">Enabled</p>
                <p className="mt-0.5 text-[10.5px] text-faint">
                  Disabled bindings stay configured but are not served to the storefront.
                </p>
              </div>
              <Switch
                checked={enabled}
                onCheckedChange={setEnabled}
                disabled={busy}
                aria-label="Capability binding enabled"
              />
            </div>
          </div>
        ) : null}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={submit} disabled={busy || !selected}>
            {busy ? "Saving…" : editing ? "Save binding" : "Bind capability"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function CapabilitiesPage() {
  const { store, mode, dataKey, can } = useAuth();
  const [editorGroup, setEditorGroup] = useState<CapabilityGroup | null>(null);
  const [unbindTarget, setUnbindTarget] = useState<CapabilityGroup | null>(null);
  const [saving, setSaving] = useState(false);
  const [unbinding, setUnbinding] = useState(false);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode !== "api" || !store) {
        return Promise.resolve<CapabilitiesResponse>({ contributions: [], bindings: [] });
      }
      return listCapabilities(store.id, signal);
    },
    [mode, store],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);
  const groups = useMemo(() => (data ? groupCapabilities(data) : []), [data]);

  async function submitBinding(
    group: CapabilityGroup,
    contribution: CapabilityContribution,
    config: JsonObject,
    enabled: boolean,
  ) {
    if (!store || mode !== "api") return;
    const rebinding = group.binding !== null;
    setSaving(true);
    try {
      await bindCapability(store.id, group.id, {
        plugin_id: contribution.plugin_id,
        version: contribution.version,
        config,
        enabled,
      });
      setEditorGroup(null);
      toast.success(rebinding ? "Capability binding updated" : "Capability bound");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  }

  async function confirmUnbind() {
    if (!unbindTarget || !store || mode !== "api") return;
    setUnbinding(true);
    try {
      await unbindCapability(store.id, unbindTarget.id);
      setUnbindTarget(null);
      toast.success("Capability unbound");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setUnbinding(false);
    }
  }

  if (mode !== "api") {
    return (
      <div className="space-y-6 animate-enter">
        <PageHeader
          title="Capabilities"
          description="Bind storefront capabilities to the plugins that provide them. Each store resolves one provider per capability."
        />
        <Card className="p-6">
          <p className="text-[13px] font-medium text-foreground">
            Connect a store to manage capabilities
          </p>
          <p className="mt-1 text-[12.5px] leading-relaxed text-muted-foreground">
            Capability bindings are stored per store and require the live API. Demo mode does not
            execute storefront plugins.
          </p>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Capabilities"
        description="Bind storefront capabilities to the plugins that provide them. Each store resolves one provider per capability."
        actions={
          <Button variant="outline" onClick={reload} disabled={loading}>
            <RefreshCw className={loading ? "size-4 animate-spin" : "size-4"} aria-hidden="true" />
            Refresh
          </Button>
        }
      />

      {error && data ? (
        <div
          role="alert"
          className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12.5px] text-destructive"
        >
          <span>{error}</span>
          <Button variant="outline" size="sm" onClick={reload}>
            Retry
          </Button>
        </div>
      ) : null}

      {!data && loading ? (
        <PageSkeleton />
      ) : !data && error ? (
        <ErrorState message={error} onRetry={reload} />
      ) : groups.length === 0 ? (
        <Card>
          <EmptyState
            title="No capabilities available"
            description="Capabilities are contributed by enabled plugins. Install or enable a plugin that declares a storefront capability."
            action={
              <Button asChild variant="outline">
                <Link to="/plugins">Manage plugins</Link>
              </Button>
            }
          />
        </Card>
      ) : (
        <>
          <section aria-label="Capability bindings" className="grid gap-3 xl:grid-cols-2">
            {groups.map((group) => (
              <CapabilityCard
                key={group.id}
                group={group}
                canWrite={can("plugins.write")}
                onEdit={() => setEditorGroup(group)}
                onUnbind={() => setUnbindTarget(group)}
              />
            ))}
          </section>
          <p className="text-[10.5px] text-faint">
            Binding changes apply to this store immediately and invalidate cached storefront
            responses.
          </p>
        </>
      )}

      <CapabilityDialog
        group={editorGroup}
        busy={saving}
        onOpenChange={(open) => {
          if (!open) setEditorGroup(null);
        }}
        onSubmit={(group, contribution, config, enabled) =>
          void submitBinding(group, contribution, config, enabled)
        }
      />

      <Dialog
        open={unbindTarget !== null}
        onOpenChange={(open) => {
          if (!open) setUnbindTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Unbind {unbindTarget?.title}?</DialogTitle>
            <DialogDescription>
              The storefront stops serving this capability for the current store. You can bind
              another provider later.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setUnbindTarget(null)} disabled={unbinding}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void confirmUnbind()} disabled={unbinding}>
              {unbinding ? "Unbinding…" : "Unbind capability"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
