import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { ArrowLeft, Check, History, Loader2, RotateCcw, Save, Send, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type {
  JsonValue,
  Theme,
  ThemeActivation,
  ThemeSettingsRevision,
} from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { ErrorState, PageSkeleton } from "@/components/states";
import { ThemeActivePill } from "@/components/status-pill";
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
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAuth } from "@/lib/auth";
import { errorMessage } from "@/lib/api";
import {
  activateDemoTheme,
  deleteDemoTheme,
  getDemoTheme,
  listDefaultThemeSettingsRevisions,
  rollbackDemoTheme,
  updateDemoThemeSettings,
} from "@/lib/demo-content";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { isJsonObject, parseJsonText, prettyJson } from "@/lib/theme-editor";
import { useAsyncData } from "@/lib/use-async-data";

const textareaClass =
  "min-h-[300px] w-full resize-y rounded-md border border-input bg-card px-3 py-2 font-mono text-[11.5px] leading-relaxed text-foreground shadow-xs outline-none transition-[color,box-shadow,border-color] placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25";

interface ThemeDetailData {
  theme: Theme;
  revisions: ThemeSettingsRevision[];
  activations: ThemeActivation[];
}

export function ThemeDetailPage() {
  const { themeId } = useParams({ from: "/themes/$themeId" });
  const navigate = useNavigate();
  const { mode, client, store, dataKey } = useAuth();
  const [settingsText, setSettingsText] = useState("");
  const [busy, setBusy] = useState<
    "save" | "draft" | "publish" | "activate" | "rollback" | "restore" | "delete" | null
  >(null);
  const [revisionsOpen, setRevisionsOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);

  const loader = useCallback(
    async (signal: AbortSignal): Promise<ThemeDetailData> => {
      if (mode === "demo") {
        const theme = getDemoTheme(themeId);
        if (!theme) throw new Error("This theme could not be found.");
        return {
          theme,
          revisions: listDefaultThemeSettingsRevisions(themeId),
          activations: [],
        };
      }
      if (!client) throw new Error("No store is selected.");
      const theme = await client.themes.get(themeId, { signal });

      const revisionsPromise: Promise<ThemeSettingsRevision[]> = client.themes.settingsRevisions
        ? client.themes.settingsRevisions(themeId, { signal }).catch(() => [])
        : Promise.resolve([]);
      const activationsPromise: Promise<ThemeActivation[]> = client.themes.activations
        ? client.themes.activations({ limit: 50 }, { signal }).catch(() => [])
        : Promise.resolve([]);
      const [revisions, activations] = await Promise.all([revisionsPromise, activationsPromise]);

      return { theme, revisions, activations };
    },
    [mode, client, themeId],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    themeId,
  ]);

  useEffect(() => {
    if (!data) return;
    setSettingsText(prettyJson(data.theme.draft_settings ?? data.theme.settings));
  }, [data]);

  const settingsBaseline = useMemo(
    () => (data ? prettyJson(data.theme.draft_settings ?? data.theme.settings) : ""),
    [data],
  );
  const parsedSettings = useMemo(() => parseJsonText(settingsText), [settingsText]);
  const settingsDirty = data !== null && settingsText !== settingsBaseline;

  const canDraft =
    mode === "demo" || Boolean(client?.themes.saveSettingsDraft ?? client?.themes.settingsDraft);
  const canPublish = mode === "demo" || Boolean(client?.themes.publishSettings);
  const canRestore = mode === "demo" || Boolean(client?.themes.restoreSettings);

  const readSettings = (): JsonValue | null => {
    const parsed = parseJsonText(settingsText);
    if (!parsed.ok) {
      toast.error(`Settings are not valid JSON: ${parsed.error ?? "invalid JSON"}`);
      return null;
    }
    return parsed.value ?? null;
  };

  const saveSettings = async () => {
    if (!data || busy) return;
    const settings = readSettings();
    if (settings === null) return;
    setBusy("save");
    try {
      if (mode === "demo") {
        updateDemoThemeSettings(themeId, settings);
        toast.success("Theme settings saved in the demo");
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.themes.updateSettings(themeId, { settings });
        reload();
        toast.success("Theme settings saved");
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const saveDraft = async () => {
    if (!data || busy) return;
    const settings = readSettings();
    if (settings === null) return;
    setBusy("draft");
    try {
      if (mode === "demo") {
        updateDemoThemeSettings(themeId, settings);
        toast.success("Settings draft saved in the demo");
      } else {
        if (!client) throw new Error("No store is selected.");
        const saveDraftMethod = client.themes.saveSettingsDraft ?? client.themes.settingsDraft;
        if (!saveDraftMethod) {
          toast.error("This backend does not support settings drafts.");
          return;
        }
        await saveDraftMethod.call(client.themes, themeId, {
          settings,
          ...(typeof data.theme.settings_revision === "number"
            ? { base_revision: data.theme.settings_revision }
            : {}),
        });
        reload();
        toast.success("Settings draft saved");
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const publishSettings = async () => {
    if (!data || busy) return;
    if (settingsDirty) {
      const settings = readSettings();
      if (settings === null) return;
      setBusy("publish");
      try {
        if (mode === "demo") {
          updateDemoThemeSettings(themeId, settings);
        } else if (client) {
          const saveDraftMethod = client.themes.saveSettingsDraft ?? client.themes.settingsDraft;
          if (saveDraftMethod) {
            await saveDraftMethod.call(client.themes, themeId, {
              settings,
              ...(typeof data.theme.settings_revision === "number"
                ? { base_revision: data.theme.settings_revision }
                : {}),
            });
          } else {
            await client.themes.updateSettings(themeId, { settings });
          }
        }
      } catch (cause) {
        toast.error(errorMessage(cause));
        setBusy(null);
        return;
      }
    }
    if (busy !== "publish") setBusy("publish");
    try {
      if (mode === "demo") {
        toast.success("Theme settings published in the demo");
      } else {
        if (!client) throw new Error("No store is selected.");
        const publishSettingsMethod = client.themes.publishSettings;
        if (!publishSettingsMethod) {
          toast.error("This backend does not support publishing settings.");
          return;
        }
        await publishSettingsMethod.call(
          client.themes,
          themeId,
          typeof data.theme.settings_revision === "number"
            ? { base_revision: data.theme.settings_revision }
            : {},
        );
        toast.success("Theme settings published");
      }
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const activate = async () => {
    if (!data || busy) return;
    setBusy("activate");
    try {
      if (mode === "demo") {
        activateDemoTheme(themeId);
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.themes.activate(themeId);
      }
      reload();
      toast.success("Theme activated");
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const rollback = async () => {
    if (busy) return;
    setBusy("rollback");
    try {
      if (mode === "demo") {
        rollbackDemoTheme();
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.themes.rollback();
      }
      reload();
      toast.success("Rolled back to the previous theme");
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!data || busy) return;
    setBusy("delete");
    try {
      if (mode === "demo") {
        deleteDemoTheme(themeId);
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.themes.delete(themeId);
      }
      toast.success("Theme deleted");
      void navigate({ to: "/themes" });
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const restoreSettings = async (revision: ThemeSettingsRevision) => {
    if (!data || busy) return;
    setBusy("restore");
    try {
      if (mode === "demo") {
        updateDemoThemeSettings(themeId, revision.settings);
      } else {
        if (!client) throw new Error("No store is selected.");
        const restore = client.themes.restoreSettings;
        if (!restore) {
          toast.error("This backend does not support restoring settings revisions.");
          return;
        }
        await restore.call(client.themes, themeId, revision.revision, {
          ...(typeof data.theme.settings_revision === "number"
            ? { base_revision: data.theme.settings_revision }
            : {}),
        });
      }
      setRevisionsOpen(false);
      reload();
      toast.success(`Restored settings revision ${revision.revision}`);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  if (!data && loading) return <PageSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const { theme } = data;
  const latestRevision = data.revisions[0] ?? null;

  return (
    <div className="space-y-6 animate-enter">
      <div>
        <Link
          to="/themes"
          className="inline-flex items-center gap-1.5 text-[11.5px] text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          All themes
        </Link>
      </div>

      <PageHeader
        title={theme.manifest.name}
        description={`${theme.manifest.id} · v${theme.manifest.version}`}
        actions={
          <>
            <div className="mr-1 hidden items-center gap-2 md:flex">
              <ThemeActivePill active={theme.is_active} />
              {settingsDirty ? <Badge variant="warning">Unsaved settings</Badge> : null}
            </div>
            <Button variant="outline" onClick={() => void rollback()} disabled={busy !== null}>
              <RotateCcw className="size-4" aria-hidden="true" />
              Roll back
            </Button>
            <Button
              variant="outline"
              onClick={() => setDeleteOpen(true)}
              disabled={busy !== null || theme.is_active}
              title={theme.is_active ? "The active theme cannot be deleted" : undefined}
            >
              <Trash2 className="size-4" aria-hidden="true" />
              Delete
            </Button>
            <Button onClick={() => void activate()} disabled={busy !== null || theme.is_active}>
              <Check className="size-4" aria-hidden="true" />
              {theme.is_active ? "Active" : "Activate"}
            </Button>
          </>
        }
      />

      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_300px]">
        <Card className="p-5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <p className="text-[13px] font-medium text-foreground">Theme settings</p>
              <p className="mt-0.5 text-[11px] text-muted-foreground">
                JSON settings validated against the theme schema.
                {typeof theme.settings_revision === "number"
                  ? ` Revision ${theme.settings_revision}.`
                  : ""}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => void saveSettings()}
                disabled={busy !== null || !settingsDirty}
              >
                {busy === "save" ? (
                  <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                ) : (
                  <Save className="size-3.5" aria-hidden="true" />
                )}
                Save settings
              </Button>
              {canDraft ? (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void saveDraft()}
                  disabled={busy !== null || !settingsDirty}
                >
                  {busy === "draft" ? (
                    <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                  ) : (
                    <Save className="size-3.5" aria-hidden="true" />
                  )}
                  Save draft
                </Button>
              ) : null}
              {canPublish ? (
                <Button size="sm" onClick={() => void publishSettings()} disabled={busy !== null}>
                  {busy === "publish" ? (
                    <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                  ) : (
                    <Send className="size-3.5" aria-hidden="true" />
                  )}
                  Publish
                </Button>
              ) : null}
            </div>
          </div>
          <textarea
            value={settingsText}
            onChange={(event) => setSettingsText(event.target.value)}
            spellCheck={false}
            aria-invalid={!parsedSettings.ok}
            className={`${textareaClass} mt-3`}
          />
          {!parsedSettings.ok ? (
            <p className="mt-1 text-[10.5px] text-destructive">
              Invalid JSON: {parsedSettings.error}
            </p>
          ) : null}
        </Card>

        <div className="space-y-4">
          <Card className="p-5">
            <div className="flex items-center justify-between gap-2">
              <p className="text-[12.5px] font-medium text-foreground">Settings history</p>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Open settings revisions"
                onClick={() => setRevisionsOpen(true)}
              >
                <History className="size-3.5" />
              </Button>
            </div>
            <p className="mt-1 text-[11px] text-muted-foreground">
              {data.revisions.length} revision{data.revisions.length === 1 ? "" : "s"} available.
            </p>
            {latestRevision ? (
              <p className="mt-2 font-mono text-[10.5px] text-faint">
                latest · r{latestRevision.revision} ·{" "}
                {shortDate(fromUnixSeconds(latestRevision.created_at))}
              </p>
            ) : null}
          </Card>
        </div>
      </div>

      <Card className="p-5">
        <Tabs defaultValue="manifest">
          <TabsList>
            <TabsTrigger value="manifest">Manifest</TabsTrigger>
            <TabsTrigger value="history">Activation history</TabsTrigger>
          </TabsList>
          <TabsContent value="manifest" className="mt-3">
            <pre className="max-h-[440px] overflow-auto rounded-lg border border-border bg-background p-3 font-mono text-[11px] leading-relaxed text-ink-soft">
              {prettyJson(theme.manifest)}
            </pre>
          </TabsContent>
          <TabsContent value="history" className="mt-3">
            {data.activations.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border px-3 py-8 text-center text-[12px] text-muted-foreground">
                No activations recorded yet.
              </p>
            ) : (
              <div className="divide-y divide-border">
                {data.activations.map((activation) => (
                  <div
                    key={activation.id}
                    className="flex items-center justify-between gap-3 py-2.5"
                  >
                    <div className="min-w-0">
                      <p className="truncate text-[12px] font-medium text-foreground">
                        {activation.manifest_id ?? activation.theme_id}
                        {activation.theme_version ? ` · v${activation.theme_version}` : ""}
                      </p>
                      <p className="truncate font-mono text-[10px] text-faint">
                        {activation.theme_id}
                        {activation.previous_theme_id
                          ? ` · from ${activation.previous_theme_id}`
                          : " · first activation"}
                      </p>
                    </div>
                    <span className="shrink-0 text-[10.5px] text-faint">
                      {shortDate(fromUnixSeconds(activation.created_at))}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </TabsContent>
        </Tabs>
      </Card>

      <Sheet open={revisionsOpen} onOpenChange={setRevisionsOpen}>
        <SheetContent side="right" className="sm:max-w-md">
          <SheetHeader>
            <SheetTitle>Settings revisions</SheetTitle>
            <SheetDescription>
              Drafts and published settings snapshots. Restoring copies a revision into the editable
              settings.
            </SheetDescription>
          </SheetHeader>
          <div className="flex-1 space-y-2 overflow-y-auto p-6">
            {data.revisions.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12px] text-muted-foreground">
                No settings revisions yet.
              </p>
            ) : (
              data.revisions.map((revision) => (
                <div key={revision.id} className="rounded-xl border border-border bg-card p-3.5">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-[12px] font-medium text-foreground">
                        r{revision.revision}
                      </span>
                      {revision.status === "published" ? (
                        <Badge variant="brand">Published</Badge>
                      ) : (
                        <Badge variant="warning">Draft</Badge>
                      )}
                    </div>
                    <span className="text-[10.5px] text-faint">
                      {shortDate(fromUnixSeconds(revision.created_at))}
                    </span>
                  </div>
                  <p className="mt-1.5 font-mono text-[10.5px] text-faint">
                    {Object.keys(isJsonObject(revision.settings) ? revision.settings : {}).length}{" "}
                    settings
                  </p>
                  <div className="mt-3 flex justify-end">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={busy !== null || !canRestore}
                      onClick={() => void restoreSettings(revision)}
                    >
                      <RotateCcw className="size-3.5" aria-hidden="true" />
                      Restore
                    </Button>
                  </div>
                </div>
              ))
            )}
          </div>
        </SheetContent>
      </Sheet>

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {theme.manifest.name}?</DialogTitle>
            <DialogDescription>
              This removes the theme registration. The active theme cannot be deleted, and pages
              authored against it are kept.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)} disabled={busy !== null}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void remove()} disabled={busy !== null}>
              Delete theme
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
