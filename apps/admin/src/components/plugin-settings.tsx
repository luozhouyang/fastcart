import { useCallback, useEffect, useState } from "react";
import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/api";
import {
  deletePluginSecret,
  listPluginSecrets,
  setPluginSecret,
  updatePluginSettings,
  type InstalledPlugin,
  type PluginSecretName,
} from "@/lib/plugins";

/**
 * JSON settings editor shared by the plugin list and the plugin detail page.
 *
 * The settings document is plugin-defined, so the editor only enforces valid
 * JSON before sending it; the API validates it against the manifest schema.
 */
export function PluginSettingsEditor({
  storeId,
  plugin,
  onSaved,
  onCancel,
  onError,
}: {
  storeId: string;
  plugin: InstalledPlugin;
  onSaved: () => void;
  onCancel: () => void;
  onError: (message: string) => void;
}) {
  const [draft, setDraft] = useState(() => JSON.stringify(plugin.settings, null, 2));
  const [busy, setBusy] = useState(false);

  async function save() {
    let parsed: unknown;
    try {
      parsed = JSON.parse(draft);
    } catch {
      onError("Settings must be valid JSON");
      return;
    }
    setBusy(true);
    try {
      await updatePluginSettings(storeId, plugin.id, parsed);
      onSaved();
    } catch (cause) {
      onError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 space-y-2">
      <textarea
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        spellCheck={false}
        rows={8}
        aria-label={`Settings for ${plugin.name}`}
        className="w-full rounded-lg border border-border bg-background p-3 font-mono text-[11.5px]"
      />
      <div className="flex gap-2">
        <Button size="sm" disabled={busy} onClick={() => void save()}>
          {busy ? <Loader2 className="size-3.5 animate-spin" aria-hidden="true" /> : null}
          Save settings
        </Button>
        <Button size="sm" variant="outline" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

/** Lists, adds and removes write-only plugin secrets. */
export function PluginSecrets({ storeId, pluginId }: { storeId: string; pluginId: string }) {
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
    <div className="space-y-2 rounded-lg border border-border bg-background p-3">
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
        <Button size="sm" disabled={busy} onClick={() => void save()}>
          Save secret
        </Button>
      </div>
    </div>
  );
}
