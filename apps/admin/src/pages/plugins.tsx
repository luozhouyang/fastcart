import { useCallback, useEffect, useRef, useState } from "react";
import { useAuth } from "@/lib/auth";
import {
  deletePlugin,
  deletePluginSecret,
  disablePlugin,
  enablePlugin,
  installPlugin,
  listPluginSecrets,
  listPlugins,
  setPluginSecret,
  updatePluginSettings,
  type InstalledPlugin,
  type PluginListResponse,
  type PluginSecretName,
} from "@/lib/plugins";

function formatDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString();
}

export function PluginsPage() {
  const { store, mode, can } = useAuth();
  const [data, setData] = useState<PluginListResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [uploading, setUploading] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [settingsDraft, setSettingsDraft] = useState("{}");
  const fileRef = useRef<HTMLInputElement>(null);

  const load = useCallback(async () => {
    if (!store || mode !== "api") return;
    try {
      setData(await listPlugins(store.id));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not load plugins");
    }
  }, [store, mode]);

  useEffect(() => {
    void load();
  }, [load]);

  async function run(id: string, action: () => Promise<unknown>) {
    setBusy(id);
    try {
      await action();
      await load();
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Plugin action failed");
    } finally {
      setBusy(null);
    }
  }

  async function upload(file: File) {
    if (!store) return;
    setUploading(true);
    try {
      await installPlugin(store.id, file);
      await load();
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Install failed");
    } finally {
      setUploading(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  if (mode !== "api") {
    return (
      <div className="rounded-xl border border-border bg-card p-6">
        <h1 className="font-display text-2xl text-foreground">Plugins</h1>
        <p className="mt-2 text-[13px] text-muted-foreground">
          Plugins require a connected store. Demo mode does not execute plugin code.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <p className="font-mono text-[10.5px] tracking-[0.14em] text-faint">EXTENSIONS</p>
          <h1 className="mt-1 font-display text-[32px] leading-none text-foreground">Plugins</h1>
          <p className="mt-2 text-[12.5px] text-muted-foreground">
            Installed extensions, their granted capabilities, and per-store state.
          </p>
        </div>
        {can("plugins.install") ? (
          <div>
            <input
              ref={fileRef}
              type="file"
              accept=".tar.gz,.tgz,application/gzip"
              className="hidden"
              onChange={(event) => {
                const file = event.target.files?.[0];
                if (file) void upload(file);
              }}
            />
            <button
              type="button"
              disabled={uploading}
              onClick={() => fileRef.current?.click()}
              className="h-9 rounded-lg bg-foreground px-3.5 text-[12.5px] font-medium text-background disabled:opacity-50"
            >
              {uploading ? "Installing…" : "Install archive"}
            </button>
          </div>
        ) : null}
      </header>

      {error ? (
        <p className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-[12.5px] text-destructive">
          {error}
        </p>
      ) : null}

      {data?.plugins.length === 0 ? (
        <div className="rounded-xl border border-dashed border-border bg-card p-8 text-center">
          <p className="text-[13px] font-medium text-foreground">No plugins installed</p>
          <p className="mt-1 text-[12px] text-muted-foreground">
            Upload a tar.gz bundle to add an extension.
          </p>
        </div>
      ) : null}

      <div className="grid gap-3">
        {data?.plugins.map((plugin) => (
          <PluginCard
            key={plugin.id}
            plugin={plugin}
            storeId={store?.id ?? ""}
            busy={busy === plugin.id}
            editing={editing === plugin.id}
            draft={settingsDraft}
            onDraft={setSettingsDraft}
            canWrite={can("plugins.write")}
            canInstall={can("plugins.install")}
            onToggle={() => {
              if (!store) return;
              void run(plugin.id, () =>
                plugin.enabled
                  ? disablePlugin(store.id, plugin.id)
                  : enablePlugin(store.id, plugin.id),
              );
            }}
            onEdit={() => {
              setEditing(plugin.id);
              setSettingsDraft(JSON.stringify(plugin.settings, null, 2));
            }}
            onCancel={() => setEditing(null)}
            onSave={() => {
              if (!store) return;
              let parsed: unknown;
              try {
                parsed = JSON.parse(settingsDraft);
              } catch {
                setError("Settings must be valid JSON");
                return;
              }
              void run(plugin.id, () => updatePluginSettings(store.id, plugin.id, parsed)).then(
                () => setEditing(null),
              );
            }}
            onDelete={() => {
              if (!store) return;
              if (!window.confirm(`Remove ${plugin.name}?`)) return;
              void run(plugin.id, () => deletePlugin(store.id, plugin.id));
            }}
          />
        ))}
      </div>

      {data && data.artifacts.length > 0 ? (
        <section className="rounded-xl border border-border bg-card p-4">
          <h2 className="text-[13px] font-medium text-foreground">Stored artifacts</h2>
          <ul className="mt-2 divide-y divide-border text-[12px]">
            {data.artifacts.map((artifact) => (
              <li key={artifact.digest} className="flex flex-wrap justify-between gap-2 py-1.5">
                <span>
                  {artifact.name} v{artifact.version}{" "}
                  <span className="font-mono text-faint">{artifact.id}</span>
                </span>
                <span className="font-mono text-[10.5px] text-faint">
                  {artifact.digest.slice(0, 12)}…
                </span>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}

interface PluginCardProps {
  plugin: InstalledPlugin;
  storeId: string;
  busy: boolean;
  editing: boolean;
  draft: string;
  onDraft: (value: string) => void;
  canWrite: boolean;
  canInstall: boolean;
  onToggle: () => void;
  onEdit: () => void;
  onCancel: () => void;
  onSave: () => void;
  onDelete: () => void;
}

function PluginCard({
  plugin,
  storeId,
  busy,
  editing,
  draft,
  onDraft,
  canWrite,
  canInstall,
  onToggle,
  onEdit,
  onCancel,
  onSave,
  onDelete,
}: PluginCardProps) {
  const [secretsOpen, setSecretsOpen] = useState(false);
  return (
    <article className="rounded-xl border border-border bg-card p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-[14px] font-medium text-foreground">{plugin.name}</h2>
            <span
              className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${
                plugin.enabled
                  ? "bg-emerald-500/10 text-emerald-700"
                  : "bg-muted text-muted-foreground"
              }`}
            >
              {plugin.enabled ? "Enabled" : "Disabled"}
            </span>
          </div>
          <p className="mt-1 font-mono text-[10.5px] text-faint">
            {plugin.id} · v{plugin.version} · {plugin.digest.slice(0, 12)}…
          </p>
          <p className="mt-1 text-[11.5px] text-muted-foreground">
            Installed {formatDate(plugin.installed_at)}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          {canWrite ? (
            <button
              type="button"
              disabled={busy}
              onClick={onToggle}
              className="h-8 rounded-lg border border-border px-3 text-[12px] hover:bg-accent disabled:opacity-50"
            >
              {plugin.enabled ? "Disable" : "Enable"}
            </button>
          ) : null}
          {canWrite ? (
            <button
              type="button"
              disabled={busy}
              onClick={onEdit}
              className="h-8 rounded-lg border border-border px-3 text-[12px] hover:bg-accent disabled:opacity-50"
            >
              Settings
            </button>
          ) : null}
          {canWrite ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => setSecretsOpen((open) => !open)}
              className="h-8 rounded-lg border border-border px-3 text-[12px] hover:bg-accent disabled:opacity-50"
            >
              Secrets
            </button>
          ) : null}
          {canInstall ? (
            <button
              type="button"
              disabled={busy || plugin.enabled}
              onClick={onDelete}
              className="h-8 rounded-lg border border-destructive/30 px-3 text-[12px] text-destructive hover:bg-destructive/5 disabled:opacity-50"
            >
              Remove
            </button>
          ) : null}
        </div>
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5">
        {plugin.granted_scopes.map((scope) => (
          <span
            key={scope}
            className="rounded-full bg-accent px-2 py-0.5 font-mono text-[10px] text-ink-soft"
          >
            {scope}
          </span>
        ))}
      </div>

      {editing ? (
        <div className="mt-3 space-y-2">
          <textarea
            value={draft}
            onChange={(event) => onDraft(event.target.value)}
            spellCheck={false}
            rows={8}
            className="w-full rounded-lg border border-border bg-background p-3 font-mono text-[11.5px]"
          />
          <div className="flex gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={onSave}
              className="h-8 rounded-lg bg-foreground px-3 text-[12px] font-medium text-background disabled:opacity-50"
            >
              Save settings
            </button>
            <button
              type="button"
              onClick={onCancel}
              className="h-8 rounded-lg border border-border px-3 text-[12px]"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : null}

      {secretsOpen && storeId ? (
        <PluginSecrets storeId={storeId} pluginId={plugin.id} />
      ) : null}
    </article>
  );
}

function PluginSecrets({ storeId, pluginId }: { storeId: string; pluginId: string }) {
  const [secrets, setSecrets] = useState<PluginSecretName[] | null>(null);
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setSecrets(await listPluginSecrets(storeId, pluginId));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not load secrets");
    }
  }, [storeId, pluginId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function save() {
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Enter a secret name.");
      return;
    }
    setBusy(true);
    try {
      await setPluginSecret(storeId, pluginId, trimmed, value);
      setName("");
      setValue("");
      await load();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not save the secret");
    } finally {
      setBusy(false);
    }
  }

  async function remove(secretName: string) {
    setBusy(true);
    try {
      await deletePluginSecret(storeId, pluginId, secretName);
      await load();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not delete the secret");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 space-y-2 rounded-lg border border-border bg-background p-3">
      <p className="text-[12px] font-medium text-foreground">Secrets</p>
      <p className="text-[11.5px] text-muted-foreground">
        Values are encrypted at rest and never shown again after saving.
      </p>
      {error ? <p className="text-[11.5px] text-destructive">{error}</p> : null}
      {secrets === null ? (
        <p className="text-[11.5px] text-muted-foreground">Loading…</p>
      ) : secrets.length === 0 ? (
        <p className="text-[11.5px] text-muted-foreground">No secrets stored.</p>
      ) : (
        <ul className="divide-y divide-border">
          {secrets.map((secret) => (
            <li
              key={secret.name}
              className="flex items-center justify-between gap-2 py-1.5 text-[12px]"
            >
              <span className="font-mono">{secret.name}</span>
              <button
                type="button"
                disabled={busy}
                onClick={() => void remove(secret.name)}
                className="text-[11.5px] text-destructive underline-offset-4 hover:underline disabled:opacity-50"
              >
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap items-end gap-2">
        <label className="flex-1 text-[11.5px] text-muted-foreground">
          Name
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            className="mt-1 h-8 w-full rounded-lg border border-border bg-card px-2 font-mono text-[12px]"
            placeholder="api_key"
          />
        </label>
        <label className="flex-[2] text-[11.5px] text-muted-foreground">
          Value
          <input
            type="password"
            value={value}
            onChange={(event) => setValue(event.target.value)}
            className="mt-1 h-8 w-full rounded-lg border border-border bg-card px-2 font-mono text-[12px]"
            autoComplete="new-password"
          />
        </label>
        <button
          type="button"
          disabled={busy}
          onClick={() => void save()}
          className="h-8 rounded-lg bg-foreground px-3 text-[12px] font-medium text-background disabled:opacity-50"
        >
          Save secret
        </button>
      </div>
    </div>
  );
}
