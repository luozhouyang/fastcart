import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import {
  ArrowLeft,
  CircleAlert,
  Loader2,
  Play,
  RefreshCw,
  RotateCcw,
  ScrollText,
  Settings2,
  ShieldCheck,
  Trash2,
  Upload,
} from "lucide-react";
import { toast } from "sonner";

import { PageHeader } from "@/components/page-header";
import { PluginSecrets, PluginSettingsEditor } from "@/components/plugin-settings";
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
import { fromUnixSeconds, shortDate, timeOfDay } from "@/lib/format";
import {
  clearLogs,
  getPlugin,
  isActiveStatus,
  listDeliveries,
  listJobs,
  listLogs,
  parseLogFields,
  replayDelivery,
  rollbackPlugin,
  runJob,
  runtimeStatus,
  statusTone,
  upgradePlugin,
  type DeliveryView,
  type JobView,
  type LogView,
  type ManifestCapabilities,
  type ManifestView,
  type PluginDetail,
  type PluginRuntimeStatus,
} from "@/lib/plugin-observability";
import { disablePlugin, enablePlugin, type InstalledPlugin } from "@/lib/plugins";
import { prettyJson } from "@/lib/theme-editor";
import { useAsyncData } from "@/lib/use-async-data";

const DEMO_CLOCK = Math.floor(new Date("2026-11-08T12:00:00Z").getTime() / 1000);

const DEMO_PLUGIN: InstalledPlugin = {
  id: "demo-insights",
  name: "Insights",
  version: "1.4.0",
  digest: "3f1c9a7d5b2e8f4a6c0d9e1b7a5f3c2d8e6b4a0f9c7d5e3b1a8f6c4d2e0b9a7f",
  previous_digest: "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899",
  enabled: true,
  settings: { greeting: "Welcome back", compact: false },
  settings_revision: 4,
  granted_scopes: ["context", "data.catalog.read", "storage.kv.read", "storage.kv.write"],
  installed_at: DEMO_CLOCK - 86_400 * 21,
  updated_at: DEMO_CLOCK - 3_600 * 5,
};

const DEMO_MANIFEST: ManifestView = {
  schema_version: "1",
  id: "demo-insights",
  name: "Insights",
  version: "1.4.0",
  description: "Surfaces storefront insight cards and a daily catalog sync job.",
  api_version: 2,
  runtime: { engine: "workerd", compatibility_date: "2026-11-01" },
  entry: { type: "worker", path: "worker/index.js" },
  capabilities: {
    data: ["catalog.read"],
    storage: ["kv.read", "kv.write"],
    secrets: ["secrets.read"],
    events: ["order.created"],
  },
  rpc: ["insights.summary", "insights.export"],
  jobs: [{ name: "catalog-sync", schedule: "0 3 * * *" }, { name: "weekly-report" }],
};

const DEMO_JOBS: JobView[] = [
  {
    id: "job_demo_2",
    plugin_id: DEMO_PLUGIN.id,
    name: "catalog-sync",
    status: "pending",
    attempts: 0,
    available_at: DEMO_CLOCK + 600,
    last_error: null,
    created_at: DEMO_CLOCK - 120,
    updated_at: DEMO_CLOCK - 120,
  },
  {
    id: "job_demo_1",
    plugin_id: DEMO_PLUGIN.id,
    name: "weekly-report",
    status: "completed",
    attempts: 1,
    available_at: DEMO_CLOCK - 3_600,
    last_error: null,
    created_at: DEMO_CLOCK - 7_200,
    updated_at: DEMO_CLOCK - 3_500,
  },
];

const DEMO_LOGS: LogView[] = [
  {
    id: 41,
    plugin_id: DEMO_PLUGIN.id,
    level: "warn",
    message: "catalog sync fell back to the cached page cursor",
    fields_json: '{"cursor":"page_7","retry_in":60}',
    created_at: DEMO_CLOCK - 900,
  },
  {
    id: 40,
    plugin_id: DEMO_PLUGIN.id,
    level: "info",
    message: "delivered order.created event to worker",
    fields_json: '{"event_id":"evt_demo_18"}',
    created_at: DEMO_CLOCK - 1_800,
  },
  {
    id: 39,
    plugin_id: DEMO_PLUGIN.id,
    level: "info",
    message: "settings revision 4 applied",
    fields_json: null,
    created_at: DEMO_CLOCK - 5_400,
  },
];

const DEMO_DELIVERIES: DeliveryView[] = [
  {
    id: "pdl_demo_3",
    plugin_id: DEMO_PLUGIN.id,
    event_id: "evt_demo_18",
    event_type: "order.created",
    status: "done",
    attempts: 1,
    available_at: DEMO_CLOCK - 1_800,
    last_error: null,
    created_at: DEMO_CLOCK - 1_800,
  },
  {
    id: "pdl_demo_2",
    plugin_id: DEMO_PLUGIN.id,
    event_id: "evt_demo_17",
    event_type: "order.created",
    status: "dead",
    attempts: 5,
    available_at: DEMO_CLOCK - 86_400,
    last_error: "worker responded with status 503",
    created_at: DEMO_CLOCK - 90_000,
  },
  {
    id: "pdl_demo_1",
    plugin_id: DEMO_PLUGIN.id,
    event_id: "evt_demo_16",
    event_type: "order.updated",
    status: "pending",
    attempts: 0,
    available_at: DEMO_CLOCK + 300,
    last_error: null,
    created_at: DEMO_CLOCK - 300,
  },
];

const DEMO_RUNTIME: PluginRuntimeStatus = {
  running: true,
  generation: "gen_demo_7",
  port: 8791,
  restarts: 1,
  last_error: null,
  uptime_seconds: 86_400 * 2 + 3_600 * 4,
};

function formatTimestamp(timestamp: number): string {
  const iso = fromUnixSeconds(timestamp);
  return `${shortDate(iso)} at ${timeOfDay(iso)}`;
}

function formatUptime(seconds: number): string {
  if (seconds <= 0) return "0s";
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0 && days === 0) parts.push(`${minutes}m`);
  return parts.length > 0 ? parts.join(" ") : `${seconds}s`;
}

function StatusBadge({ status }: { status: string }) {
  return (
    <Badge variant={statusTone(status)} className="capitalize">
      {status.replaceAll("_", " ")}
    </Badge>
  );
}

function LevelBadge({ level }: { level: string }) {
  const normalized = level.toLowerCase();
  const tone =
    normalized === "error"
      ? "destructive"
      : normalized === "warn" || normalized === "warning"
        ? "warning"
        : normalized === "debug"
          ? "secondary"
          : "info";
  return (
    <Badge variant={tone} className="uppercase">
      {level}
    </Badge>
  );
}

function ErrorBanner({ message }: { message: string }) {
  return (
    <p className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12.5px] text-destructive">
      {message}
    </p>
  );
}

function OverviewTab({
  detail,
  canWrite,
  storeId,
  onChanged,
  onError,
}: {
  detail: PluginDetail;
  canWrite: boolean;
  /** Empty in demo mode, where settings and secrets are read-only fixtures. */
  storeId: string;
  onChanged: () => void;
  onError: (message: string) => void;
}) {
  const { plugin, manifest } = detail;
  const editable = canWrite && storeId.length > 0;
  const [editingSettings, setEditingSettings] = useState(false);
  const [secretsOpen, setSecretsOpen] = useState(false);

  const declaredScopes = useMemo(() => {
    const capabilities: ManifestCapabilities = manifest.capabilities ?? {};
    return (Object.entries(capabilities) as Array<[string, string[] | undefined]>).flatMap(
      ([group, scopes]) => (scopes ?? []).map((scope) => `${group}.${scope}`),
    );
  }, [manifest]);

  return (
    <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
      <Card className="gap-4 p-5">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div>
            <p className="text-[13px] font-medium text-foreground">Settings</p>
            <p className="mt-0.5 text-[11.5px] text-muted-foreground">
              Revision {plugin.settings_revision} · validated against the manifest schema.
            </p>
          </div>
          {editable ? (
            <Button variant="outline" size="sm" onClick={() => setEditingSettings((open) => !open)}>
              <Settings2 className="size-3.5" aria-hidden="true" />
              {editingSettings ? "Close editor" : "Edit settings"}
            </Button>
          ) : null}
        </div>
        {editingSettings && editable ? (
          <PluginSettingsEditor
            storeId={storeId}
            plugin={plugin}
            onSaved={() => {
              setEditingSettings(false);
              onChanged();
            }}
            onCancel={() => setEditingSettings(false)}
            onError={onError}
          />
        ) : (
          <pre className="max-h-72 overflow-auto rounded-lg border border-border bg-background p-3 font-mono text-[11px] leading-relaxed text-ink-soft">
            {prettyJson(plugin.settings)}
          </pre>
        )}

        <div className="mt-2">
          <div className="flex items-center justify-between gap-2">
            <p className="text-[13px] font-medium text-foreground">Secrets</p>
            {editable ? (
              <Button variant="ghost" size="sm" onClick={() => setSecretsOpen((open) => !open)}>
                {secretsOpen ? "Hide" : "Manage secrets"}
              </Button>
            ) : null}
          </div>
          {secretsOpen && editable ? (
            <div className="mt-2">
              <PluginSecrets storeId={storeId} pluginId={plugin.id} />
            </div>
          ) : (
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Secret values are write-only and never returned by the API.
            </p>
          )}
        </div>
      </Card>

      <div className="space-y-4">
        <Card className="gap-3 p-5">
          <p className="text-[13px] font-medium text-foreground">Artifact</p>
          <dl className="space-y-2 text-[11.5px]">
            <div>
              <dt className="text-muted-foreground">Version</dt>
              <dd className="mt-0.5 font-mono text-[11px] text-ink-soft">
                v{plugin.version} · settings revision {plugin.settings_revision}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Active digest</dt>
              <dd className="mt-0.5 font-mono text-[10.5px] break-all text-ink-soft">
                {plugin.digest}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Rollback digest</dt>
              <dd className="mt-0.5 font-mono text-[10.5px] break-all text-ink-soft">
                {plugin.previous_digest ?? "None available"}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Runtime</dt>
              <dd className="mt-0.5 text-ink-soft">
                {manifest.runtime.engine} · {manifest.runtime.compatibility_date}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Entry</dt>
              <dd className="mt-0.5 font-mono text-[10.5px] text-ink-soft">
                {manifest.entry.path}
              </dd>
            </div>
          </dl>
        </Card>

        <Card className="gap-3 p-5">
          <div className="flex items-center gap-2">
            <ShieldCheck className="size-4 text-brand-ink" aria-hidden="true" />
            <p className="text-[13px] font-medium text-foreground">Granted scopes</p>
          </div>
          {plugin.granted_scopes.length === 0 ? (
            <p className="text-[11.5px] text-muted-foreground">No scopes granted.</p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {plugin.granted_scopes.map((scope) => (
                <span
                  key={scope}
                  className="rounded-full bg-accent px-2 py-0.5 font-mono text-[10px] text-ink-soft"
                >
                  {scope}
                </span>
              ))}
            </div>
          )}
          {declaredScopes.length > 0 ? (
            <>
              <p className="mt-1 text-[10.5px] font-medium tracking-[0.08em] text-faint uppercase">
                Declared by manifest
              </p>
              <div className="flex flex-wrap gap-1.5">
                {declaredScopes.map((scope) => (
                  <span
                    key={scope}
                    className="rounded-full border border-border px-2 py-0.5 font-mono text-[10px] text-muted-foreground"
                  >
                    {scope}
                  </span>
                ))}
              </div>
            </>
          ) : null}
        </Card>

        {manifest.rpc && manifest.rpc.length > 0 ? (
          <Card className="gap-3 p-5">
            <p className="text-[13px] font-medium text-foreground">RPC methods</p>
            <div className="flex flex-wrap gap-1.5">
              {manifest.rpc.map((method) => (
                <span
                  key={method}
                  className="rounded-full bg-accent px-2 py-0.5 font-mono text-[10px] text-ink-soft"
                >
                  {method}
                </span>
              ))}
            </div>
          </Card>
        ) : null}

        <Card className="gap-3 p-5">
          <p className="text-[13px] font-medium text-foreground">Manifest</p>
          <pre className="max-h-80 overflow-auto rounded-lg border border-border bg-background p-3 font-mono text-[10.5px] leading-relaxed text-ink-soft">
            {prettyJson(manifest)}
          </pre>
        </Card>
      </div>
    </div>
  );
}

function JobsTab({
  jobs,
  declared,
  canWrite,
  busy,
  onRun,
}: {
  jobs: JobView[];
  declared: Array<{ name: string; schedule?: string | null }>;
  canWrite: boolean;
  busy: string | null;
  onRun: (name: string) => void;
}) {
  const runsByName = new Map(jobs.map((job) => [job.name, job]));

  return (
    <div className="space-y-4">
      {declared.length > 0 ? (
        <Card className="gap-3 p-4">
          <p className="text-[12.5px] font-medium text-foreground">Declared jobs</p>
          <div className="flex flex-wrap gap-2">
            {declared.map((job) => (
              <span
                key={job.name}
                className="inline-flex items-center gap-2 rounded-full border border-border bg-background px-2.5 py-1 text-[11.5px]"
              >
                <span className="font-mono">{job.name}</span>
                {job.schedule ? (
                  <span className="font-mono text-[10px] text-faint">{job.schedule}</span>
                ) : (
                  <span className="text-[10px] text-faint">manual</span>
                )}
                {canWrite ? (
                  <button
                    type="button"
                    disabled={busy === job.name}
                    onClick={() => onRun(job.name)}
                    className="inline-flex items-center gap-1 text-[10.5px] text-brand-ink underline-offset-4 hover:underline disabled:opacity-50"
                  >
                    {busy === job.name ? (
                      <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                    ) : (
                      <Play className="size-3" aria-hidden="true" />
                    )}
                    Run now
                  </button>
                ) : null}
              </span>
            ))}
          </div>
          {runsByName.size === 0 ? (
            <p className="text-[11px] text-muted-foreground">
              No job runs recorded yet. Running a job enqueues it immediately.
            </p>
          ) : null}
        </Card>
      ) : null}

      {jobs.length === 0 ? (
        <EmptyState
          title="No job runs yet"
          description="Jobs appear here after the scheduler or an admin enqueues them."
        />
      ) : (
        <Card className="overflow-hidden p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Job</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Attempts</TableHead>
                <TableHead>Available</TableHead>
                <TableHead>Last error</TableHead>
                <TableHead className="text-right">Action</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {jobs.map((job) => (
                <TableRow key={job.id}>
                  <TableCell>
                    <span className="font-mono text-[12px] text-foreground">{job.name}</span>
                    <span className="mt-0.5 block font-mono text-[10px] text-faint">{job.id}</span>
                  </TableCell>
                  <TableCell>
                    <StatusBadge status={job.status} />
                  </TableCell>
                  <TableCell className="text-[12px] text-ink-soft tnum">{job.attempts}</TableCell>
                  <TableCell className="text-[11.5px] text-ink-soft">
                    {formatTimestamp(job.available_at)}
                  </TableCell>
                  <TableCell className="max-w-[220px] text-[11.5px] text-destructive">
                    <span className="line-clamp-2 break-words">{job.last_error ?? "—"}</span>
                  </TableCell>
                  <TableCell className="text-right">
                    {canWrite ? (
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={busy === job.name}
                        onClick={() => onRun(job.name)}
                      >
                        {busy === job.name ? (
                          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                        ) : (
                          <Play className="size-3.5" aria-hidden="true" />
                        )}
                        Run now
                      </Button>
                    ) : null}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>
      )}
    </div>
  );
}

function LogsTab({
  logs,
  canWrite,
  onClear,
}: {
  logs: LogView[];
  canWrite: boolean;
  onClear: () => void;
}) {
  return (
    <Card className="gap-3 p-5">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-[13px] font-medium text-foreground">Recent log lines</p>
          <p className="mt-0.5 text-[11.5px] text-muted-foreground">
            The API keeps the latest 200 lines per plugin.
          </p>
        </div>
        {canWrite ? (
          <Button variant="outline" size="sm" disabled={logs.length === 0} onClick={onClear}>
            <Trash2 className="size-3.5" aria-hidden="true" />
            Clear logs
          </Button>
        ) : null}
      </div>

      {logs.length === 0 ? (
        <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-border px-4 py-10 text-center">
          <ScrollText className="size-5 text-faint" aria-hidden="true" />
          <p className="text-[12.5px] text-muted-foreground">No log lines recorded.</p>
        </div>
      ) : (
        <ul className="divide-y divide-border">
          {logs.map((log) => {
            const fields = parseLogFields(log.fields_json);
            return (
              <li key={log.id} className="py-2.5">
                <div className="flex flex-wrap items-center gap-2">
                  <LevelBadge level={log.level} />
                  <span className="font-mono text-[10px] text-faint">
                    {formatTimestamp(log.created_at)}
                  </span>
                </div>
                <p className="mt-1 text-[12px] text-foreground">{log.message}</p>
                {fields ? (
                  <pre className="mt-1 overflow-x-auto rounded-md bg-secondary/60 px-2 py-1 font-mono text-[10.5px] text-ink-soft">
                    {prettyJson(fields)}
                  </pre>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
    </Card>
  );
}

function DeliveriesTab({
  deliveries,
  canWrite,
  busy,
  onReplay,
}: {
  deliveries: DeliveryView[];
  canWrite: boolean;
  busy: string | null;
  onReplay: (deliveryId: string) => void;
}) {
  if (deliveries.length === 0) {
    return (
      <EmptyState
        title="No deliveries yet"
        description="Event deliveries appear here when a subscribed event reaches the plugin."
      />
    );
  }

  return (
    <Card className="overflow-hidden p-0">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Event</TableHead>
            <TableHead>Status</TableHead>
            <TableHead>Attempts</TableHead>
            <TableHead>Created</TableHead>
            <TableHead>Last error</TableHead>
            <TableHead className="text-right">Action</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {deliveries.map((delivery) => (
            <TableRow key={delivery.id}>
              <TableCell>
                <span className="font-mono text-[12px] text-foreground">{delivery.event_type}</span>
                <span className="mt-0.5 block font-mono text-[10px] text-faint">
                  {delivery.event_id}
                </span>
              </TableCell>
              <TableCell>
                <StatusBadge status={delivery.status} />
              </TableCell>
              <TableCell className="text-[12px] text-ink-soft tnum">{delivery.attempts}</TableCell>
              <TableCell className="text-[11.5px] text-ink-soft">
                {formatTimestamp(delivery.created_at)}
              </TableCell>
              <TableCell className="max-w-[220px] text-[11.5px] text-destructive">
                <span className="line-clamp-2 break-words">{delivery.last_error ?? "—"}</span>
              </TableCell>
              <TableCell className="text-right">
                {canWrite ? (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy === delivery.id || isActiveStatus(delivery.status)}
                    onClick={() => onReplay(delivery.id)}
                    title={
                      isActiveStatus(delivery.status)
                        ? "This delivery is already queued"
                        : undefined
                    }
                  >
                    {busy === delivery.id ? (
                      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                    ) : (
                      <RotateCcw className="size-3.5" aria-hidden="true" />
                    )}
                    Replay
                  </Button>
                ) : null}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </Card>
  );
}

function RuntimeTab({
  status,
  loading,
  onRefresh,
}: {
  status: PluginRuntimeStatus;
  loading: boolean;
  onRefresh: () => void;
}) {
  const rows: Array<{ label: string; value: string; mono?: boolean }> = [
    { label: "Generation", value: status.generation ?? "—", mono: true },
    { label: "Port", value: status.port === null ? "—" : String(status.port), mono: true },
    { label: "Restarts", value: String(status.restarts) },
    { label: "Uptime", value: formatUptime(status.uptime_seconds) },
  ];

  return (
    <div className="space-y-4">
      <Card className="gap-4 p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex items-center gap-3">
            <span
              className={`flex size-9 items-center justify-center rounded-full ${
                status.running
                  ? "bg-emerald-500/10 text-emerald-700"
                  : "bg-destructive/10 text-destructive"
              }`}
            >
              {status.running ? (
                <ShieldCheck className="size-4" aria-hidden="true" />
              ) : (
                <CircleAlert className="size-4" aria-hidden="true" />
              )}
            </span>
            <div>
              <p className="text-[13px] font-medium text-foreground">
                {status.running ? "Runtime running" : "Runtime stopped"}
              </p>
              <p className="mt-0.5 text-[11.5px] text-muted-foreground">
                Shared workerd process serving every installed plugin.
              </p>
            </div>
          </div>
          <Button variant="outline" size="sm" onClick={onRefresh} disabled={loading}>
            {loading ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw className="size-3.5" aria-hidden="true" />
            )}
            Refresh
          </Button>
        </div>

        <dl className="grid gap-3 sm:grid-cols-2">
          {rows.map((row) => (
            <div
              key={row.label}
              className="rounded-lg border border-border bg-background px-3 py-2"
            >
              <dt className="text-[10.5px] font-medium tracking-[0.08em] text-faint uppercase">
                {row.label}
              </dt>
              <dd
                className={`mt-0.5 text-[12.5px] text-foreground ${
                  row.mono ? "font-mono text-[11.5px]" : ""
                }`}
              >
                {row.value}
              </dd>
            </div>
          ))}
        </dl>

        {status.last_error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[11.5px] text-destructive">
            {status.last_error}
          </p>
        ) : (
          <p className="text-[11.5px] text-muted-foreground">No runtime errors reported.</p>
        )}
      </Card>
    </div>
  );
}

interface UpgradeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pluginName: string;
  busy: boolean;
  onUpload: (archive: File, acceptNewScopes: boolean) => void;
}

function UpgradeDialog({ open, onOpenChange, pluginName, busy, onUpload }: UpgradeDialogProps) {
  const [archive, setArchive] = useState<File | null>(null);
  const [acceptNewScopes, setAcceptNewScopes] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) {
      setArchive(null);
      setAcceptNewScopes(false);
    }
  }, [open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Upgrade {pluginName}</DialogTitle>
          <DialogDescription>
            Upload a tar.gz bundle built for this plugin id. The previous artifact stays available
            for rollback.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <input
            ref={inputRef}
            type="file"
            accept=".tar.gz,.tgz,application/gzip"
            className="block w-full text-[12px] text-ink-soft file:mr-3 file:rounded-lg file:border file:border-border file:bg-card file:px-3 file:py-1.5 file:text-[12px] file:text-foreground"
            onChange={(event) => setArchive(event.target.files?.[0] ?? null)}
          />
          <label className="flex items-start gap-2.5 rounded-lg border border-border bg-background p-3">
            <Switch
              checked={acceptNewScopes}
              onCheckedChange={setAcceptNewScopes}
              aria-label="Accept new capabilities"
            />
            <span>
              <span className="block text-[12px] text-foreground">Accept new capabilities</span>
              <span className="mt-0.5 block text-[11px] text-muted-foreground">
                Required when the new version requests scopes the store has not granted yet.
              </span>
            </span>
          </label>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button
            disabled={busy || archive === null}
            onClick={() => {
              if (archive) onUpload(archive, acceptNewScopes);
            }}
          >
            {busy ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Upload className="size-4" aria-hidden="true" />
            )}
            Upload upgrade
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface DetailData {
  detail: PluginDetail;
  jobs: JobView[];
  logs: LogView[];
  deliveries: DeliveryView[];
  runtime: PluginRuntimeStatus;
}

export function PluginDetailPage() {
  const params = useParams({ strict: false }) as { pluginId?: string };
  const pluginId = params.pluginId ?? "";
  const { mode, store, dataKey, can } = useAuth();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [rollbackOpen, setRollbackOpen] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [upgradeOpen, setUpgradeOpen] = useState(false);

  // Demo mode keeps a local copy so every tab still renders without an API.
  const [demoPlugin, setDemoPlugin] = useState<InstalledPlugin>(DEMO_PLUGIN);
  const [demoJobs, setDemoJobs] = useState<JobView[]>(DEMO_JOBS);
  const [demoLogs, setDemoLogs] = useState<LogView[]>(DEMO_LOGS);
  const [demoDeliveries, setDemoDeliveries] = useState<DeliveryView[]>(DEMO_DELIVERIES);

  const loader = useCallback(
    async (signal: AbortSignal): Promise<DetailData> => {
      if (mode === "demo") {
        return {
          detail: { plugin: DEMO_PLUGIN, manifest: DEMO_MANIFEST },
          jobs: DEMO_JOBS,
          logs: DEMO_LOGS,
          deliveries: DEMO_DELIVERIES,
          runtime: DEMO_RUNTIME,
        };
      }
      if (!store) throw new Error("No store is selected.");
      const [detail, jobs, logs, deliveries, runtime] = await Promise.all([
        getPlugin(store.id, pluginId, signal),
        listJobs(store.id, pluginId, signal),
        listLogs(store.id, pluginId, signal),
        listDeliveries(store.id, pluginId, signal),
        runtimeStatus(store.id, signal),
      ]);
      return { detail, jobs, logs, deliveries, runtime };
    },
    [mode, store, pluginId],
  );

  const {
    data,
    error: loadError,
    loading,
    reload,
  } = useAsyncData(loader, [mode, store?.id, dataKey, pluginId]);

  const isDemo = mode === "demo";
  const canWrite = can("plugins.write");
  const canInstall = can("plugins.install");

  // The demo data is local state; the API data comes from the loader. Both
  // paths converge on one view model for the header and tabs.
  const demoDetail: PluginDetail = useMemo(
    () => ({ plugin: demoPlugin, manifest: DEMO_MANIFEST }),
    [demoPlugin],
  );
  const view: DetailData | null = isDemo
    ? {
        detail: demoDetail,
        jobs: demoJobs,
        logs: demoLogs,
        deliveries: demoDeliveries,
        runtime: DEMO_RUNTIME,
      }
    : data;

  const plugin = view?.detail.plugin ?? null;

  useEffect(() => {
    setError(null);
  }, [pluginId]);

  if (!pluginId) {
    return <ErrorState message="No plugin id was provided." />;
  }
  if (!view) {
    if (loading) return <PageSkeleton />;
    if (loadError) {
      return <ErrorState message={loadError} onRetry={reload} />;
    }
    return null;
  }

  const refresh = () => {
    if (isDemo) return;
    reload();
  };

  const toggleEnabled = async () => {
    if (actionBusy || !plugin) return;
    if (isDemo) {
      setDemoPlugin((current) => ({ ...current, enabled: !current.enabled }));
      toast.success(plugin.enabled ? "Plugin disabled" : "Plugin enabled");
      return;
    }
    if (!store) return;
    setActionBusy(true);
    try {
      if (plugin.enabled) await disablePlugin(store.id, plugin.id);
      else await enablePlugin(store.id, plugin.id);
      toast.success(plugin.enabled ? "Plugin disabled" : "Plugin enabled");
      refresh();
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setActionBusy(false);
    }
  };

  const rollback = async () => {
    if (actionBusy) return;
    setActionBusy(true);
    try {
      if (isDemo) {
        setDemoPlugin((current) => ({
          ...current,
          version: current.previous_digest ? "1.3.2" : current.version,
          digest: current.previous_digest ?? current.digest,
          previous_digest: current.digest,
          updated_at: DEMO_CLOCK,
        }));
      } else {
        if (!store || !plugin) return;
        await rollbackPlugin(store.id, plugin.id);
        refresh();
      }
      setRollbackOpen(false);
      toast.success("Rolled back to the previous artifact");
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setActionBusy(false);
    }
  };

  const upgrade = async (archive: File, acceptNewScopes: boolean) => {
    if (actionBusy) return;
    setActionBusy(true);
    try {
      if (isDemo) {
        setDemoPlugin((current) => ({
          ...current,
          version: "1.5.0",
          previous_digest: current.digest,
          digest: "b7e4c1a9d3f6e2b8c5a0d7f4e1b9c6a3d8f5e2b0c7a4d1f8e6b3c9a5d2f7e4b1",
          updated_at: DEMO_CLOCK,
        }));
      } else {
        if (!store || !plugin) return;
        await upgradePlugin(store.id, plugin.id, archive, acceptNewScopes);
        refresh();
      }
      setUpgradeOpen(false);
      toast.success("Plugin upgraded");
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setActionBusy(false);
    }
  };

  const runDeclaredJob = async (name: string) => {
    if (busy !== null) return;
    setBusy(name);
    try {
      if (isDemo) {
        const job: JobView = {
          id: `job_demo_${Date.now()}`,
          plugin_id: plugin?.id ?? pluginId,
          name,
          status: "pending",
          attempts: 0,
          available_at: DEMO_CLOCK,
          last_error: null,
          created_at: DEMO_CLOCK,
          updated_at: DEMO_CLOCK,
        };
        setDemoJobs((current) => [job, ...current]);
      } else {
        if (!store) return;
        await runJob(store.id, pluginId, name);
        toast.success(`Queued ${name}`);
        refresh();
      }
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const clearAllLogs = async () => {
    if (actionBusy) return;
    setActionBusy(true);
    try {
      if (isDemo) {
        setDemoLogs([]);
      } else {
        if (!store) return;
        await clearLogs(store.id, pluginId);
        refresh();
      }
      setClearOpen(false);
      toast.success("Plugin logs cleared");
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setActionBusy(false);
    }
  };

  const replay = async (deliveryId: string) => {
    if (busy !== null) return;
    setBusy(deliveryId);
    try {
      if (isDemo) {
        setDemoDeliveries((current) =>
          current.map((delivery) =>
            delivery.id === deliveryId
              ? { ...delivery, status: "pending", attempts: 0, last_error: null }
              : delivery,
          ),
        );
      } else {
        if (!store) return;
        await replayDelivery(store.id, pluginId, deliveryId);
        toast.success("Delivery queued again");
        refresh();
      }
      setError(null);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const manifest = view.detail.manifest;
  const storeId = store?.id ?? "";

  return (
    <div className="space-y-6 animate-enter">
      <div>
        <Link
          to="/plugins"
          className="inline-flex items-center gap-1.5 text-[11.5px] text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          All plugins
        </Link>
      </div>

      <PageHeader
        title={plugin?.name ?? manifest.name}
        description={`${plugin?.id ?? manifest.id} · v${plugin?.version ?? manifest.version}${
          plugin ? ` · revision ${plugin.settings_revision}` : ""
        }`}
        actions={
          <>
            <div className="mr-1 hidden items-center gap-2 md:flex">
              <Badge variant={plugin?.enabled ? "brand" : "secondary"}>
                {plugin?.enabled ? "Enabled" : "Disabled"}
              </Badge>
              {isDemo ? <Badge variant="outline">Demo</Badge> : null}
            </div>
            {canInstall ? (
              <Button variant="outline" onClick={() => setUpgradeOpen(true)} disabled={actionBusy}>
                <Upload className="size-4" aria-hidden="true" />
                Upgrade
              </Button>
            ) : null}
            {canWrite ? (
              <Button
                variant="outline"
                onClick={() => setRollbackOpen(true)}
                disabled={actionBusy || !plugin?.previous_digest}
                title={
                  plugin?.previous_digest
                    ? "Restore the previous artifact"
                    : "No previous artifact is available"
                }
              >
                <RotateCcw className="size-4" aria-hidden="true" />
                Roll back
              </Button>
            ) : null}
            {canWrite ? (
              <Button onClick={() => void toggleEnabled()} disabled={actionBusy}>
                {actionBusy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
                {plugin?.enabled ? "Disable" : "Enable"}
              </Button>
            ) : null}
          </>
        }
      />

      {error ? <ErrorBanner message={error} /> : null}

      <Card className="p-5">
        <Tabs defaultValue="overview">
          <TabsList>
            <TabsTrigger value="overview">Overview</TabsTrigger>
            <TabsTrigger value="jobs">Jobs</TabsTrigger>
            <TabsTrigger value="logs">Logs</TabsTrigger>
            <TabsTrigger value="deliveries">Deliveries</TabsTrigger>
            <TabsTrigger value="runtime">Runtime</TabsTrigger>
          </TabsList>

          <TabsContent value="overview" className="mt-4">
            <OverviewTab
              detail={view.detail}
              canWrite={canWrite}
              storeId={isDemo ? "" : storeId}
              onChanged={refresh}
              onError={setError}
            />
          </TabsContent>

          <TabsContent value="jobs" className="mt-4">
            <JobsTab
              jobs={view.jobs}
              declared={manifest.jobs ?? []}
              canWrite={canWrite}
              busy={busy}
              onRun={(name) => void runDeclaredJob(name)}
            />
          </TabsContent>

          <TabsContent value="logs" className="mt-4">
            <LogsTab logs={view.logs} canWrite={canWrite} onClear={() => setClearOpen(true)} />
          </TabsContent>

          <TabsContent value="deliveries" className="mt-4">
            <DeliveriesTab
              deliveries={view.deliveries}
              canWrite={canWrite}
              busy={busy}
              onReplay={(deliveryId) => void replay(deliveryId)}
            />
          </TabsContent>

          <TabsContent value="runtime" className="mt-4">
            <RuntimeTab status={view.runtime} loading={loading} onRefresh={refresh} />
          </TabsContent>
        </Tabs>
      </Card>

      <Dialog open={rollbackOpen} onOpenChange={setRollbackOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Roll back {plugin?.name ?? pluginId}?</DialogTitle>
            <DialogDescription>
              The plugin switches to its previously active artifact. The current artifact stays
              available, so a second rollback returns to it.
            </DialogDescription>
          </DialogHeader>
          <p className="rounded-lg border border-border bg-background px-3 py-2 font-mono text-[10.5px] break-all text-ink-soft">
            {plugin?.digest} → {plugin?.previous_digest}
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRollbackOpen(false)} disabled={actionBusy}>
              Cancel
            </Button>
            <Button onClick={() => void rollback()} disabled={actionBusy}>
              {actionBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <RotateCcw className="size-4" aria-hidden="true" />
              )}
              Roll back
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={clearOpen} onOpenChange={setClearOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Clear plugin logs?</DialogTitle>
            <DialogDescription>
              This deletes every stored log line for {plugin?.name ?? pluginId}. It cannot be
              undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setClearOpen(false)} disabled={actionBusy}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void clearAllLogs()} disabled={actionBusy}>
              {actionBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Clear logs
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <UpgradeDialog
        open={upgradeOpen}
        onOpenChange={setUpgradeOpen}
        pluginName={plugin?.name ?? pluginId}
        busy={actionBusy}
        onUpload={(archive, acceptNewScopes) => void upgrade(archive, acceptNewScopes)}
      />
    </div>
  );
}
