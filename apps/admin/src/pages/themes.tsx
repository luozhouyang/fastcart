import { useCallback, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowUpRight, History, MoreHorizontal, Plus, RotateCcw, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { JsonValue, RegisterThemeRequest, Theme, ThemeManifest } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { ThemeActivePill } from "@/components/status-pill";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAuth } from "@/lib/auth";
import { errorMessage } from "@/lib/api";
import {
  activateDemoTheme,
  deleteDemoTheme,
  listDemoThemes,
  registerDemoTheme,
  rollbackDemoTheme,
} from "@/lib/demo-content";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { isJsonObject } from "@/lib/theme-editor";
import { useAsyncData } from "@/lib/use-async-data";

const textareaClass =
  "min-h-[110px] w-full resize-y rounded-md border border-input bg-card px-3 py-2 font-mono text-[11.5px] leading-relaxed text-foreground shadow-xs outline-none transition-[color,box-shadow,border-color] placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25";

type ParsedOptional = { ok: true; value: JsonValue | undefined } | { ok: false; error: string };

function parseOptionalJson(text: string, label: string): ParsedOptional {
  const trimmed = text.trim();
  if (trimmed.length === 0) return { ok: true, value: undefined };
  try {
    return { ok: true, value: JSON.parse(trimmed) as JsonValue };
  } catch {
    return { ok: false, error: `${label} is not valid JSON.` };
  }
}

interface RegisterForm {
  id: string;
  name: string;
  version: string;
  runtime: string;
  entry: string;
  bundlePath: string;
  manifestJson: string;
  settingsSchema: string;
  settings: string;
  editorManifest: string;
}

const EMPTY_FORM: RegisterForm = {
  id: "",
  name: "",
  version: "1.0.0",
  runtime: "node",
  entry: "dist/server/server.js",
  bundlePath: "",
  manifestJson: "",
  settingsSchema: "",
  settings: "",
  editorManifest: "",
};

function buildRegisterRequest(
  form: RegisterForm,
  mode: "fields" | "json",
): { ok: true; request: RegisterThemeRequest } | { ok: false; error: string } {
  const schema = parseOptionalJson(form.settingsSchema, "Settings schema");
  if (!schema.ok) return schema;
  const editor = parseOptionalJson(form.editorManifest, "Editor manifest");
  if (!editor.ok) return editor;
  const settings = parseOptionalJson(form.settings, "Initial settings");
  if (!settings.ok) return settings;

  let manifest: ThemeManifest;
  if (mode === "json") {
    const parsed = parseOptionalJson(form.manifestJson, "Manifest");
    if (!parsed.ok) return parsed;
    if (!isJsonObject(parsed.value)) {
      return { ok: false, error: "Paste a JSON object for the theme manifest." };
    }
    const record = parsed.value;
    for (const key of ["id", "name", "version", "runtime", "entry"] as const) {
      if (typeof record[key] !== "string" || (record[key] as string).trim().length === 0) {
        return { ok: false, error: `Manifest field "${key}" is required.` };
      }
    }
    manifest = record as unknown as ThemeManifest;
  } else {
    const id = form.id.trim();
    const name = form.name.trim();
    if (id.length === 0) return { ok: false, error: "Theme id is required." };
    if (name.length === 0) return { ok: false, error: "Theme name is required." };
    if (form.version.trim().length === 0) {
      return { ok: false, error: "Theme version is required." };
    }
    if (form.runtime.trim().length === 0) {
      return { ok: false, error: "Runtime is required." };
    }
    if (form.entry.trim().length === 0) {
      return { ok: false, error: "Entry file is required." };
    }
    manifest = {
      id,
      name,
      version: form.version.trim(),
      runtime: form.runtime.trim(),
      entry: form.entry.trim(),
      settings_schema: null,
      assets_manifest: null,
    };
  }

  const bundlePath = form.bundlePath.trim() || manifest.id;

  // The backend rejects an editor manifest unless the theme manifest also
  // declares an editor reference. Infer a conventional reference so pasting an
  // editor manifest alone is enough to register the theme.
  if (editor.value !== undefined && (manifest.editor === undefined || manifest.editor === null)) {
    const schemaVersion =
      isJsonObject(editor.value) && typeof editor.value.schema_version === "number"
        ? editor.value.schema_version
        : 1;
    manifest = {
      ...manifest,
      editor: {
        entry: "dist/client/editor.js",
        manifest: "editor.manifest.json",
        schema_version: schemaVersion,
      },
    };
  }

  const request: RegisterThemeRequest = {
    manifest,
    bundle_path: bundlePath,
    ...(schema.value !== undefined ? { settings_schema: schema.value } : {}),
    ...(settings.value !== undefined ? { settings: settings.value } : {}),
    ...(editor.value !== undefined ? { editor_manifest: editor.value } : {}),
  };
  return { ok: true, request };
}

export function ThemesPage() {
  const { mode, client, store, dataKey } = useAuth();
  const navigate = useNavigate();
  const [demoThemes, setDemoThemes] = useState<Theme[]>(() => listDemoThemes());
  const [registerOpen, setRegisterOpen] = useState(false);
  const [form, setForm] = useState<RegisterForm>(EMPTY_FORM);
  const [manifestMode, setManifestMode] = useState<"fields" | "json">("fields");
  const [busy, setBusy] = useState<"register" | "activate" | "rollback" | "delete" | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Theme | null>(null);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") return Promise.resolve(listDemoThemes());
      if (!client) return Promise.reject(new Error("No store is selected."));
      return client.themes.list({ limit: 100 }, { signal }).then((page) => page.data);
    },
    [mode, client],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);
  const themes = mode === "demo" ? demoThemes : (data ?? []);

  const refreshDemoThemes = () => setDemoThemes(listDemoThemes());

  const openRegister = () => {
    setForm(EMPTY_FORM);
    setManifestMode("fields");
    setRegisterOpen(true);
  };

  const register = async () => {
    if (busy) return;
    const built = buildRegisterRequest(form, manifestMode);
    if (!built.ok) {
      toast.error(built.error);
      return;
    }
    setBusy("register");
    try {
      if (mode === "demo") {
        registerDemoTheme(built.request);
        refreshDemoThemes();
        toast.success("Theme registered in the demo");
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.themes.register(built.request);
        reload();
        toast.success("Theme registered");
      }
      setRegisterOpen(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const activate = async (theme: Theme) => {
    if (busy) return;
    setBusy("activate");
    try {
      if (mode === "demo") {
        activateDemoTheme(theme.id);
        refreshDemoThemes();
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.themes.activate(theme.id);
        reload();
      }
      toast.success(`${theme.manifest.name} is now active`);
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
        const theme = rollbackDemoTheme();
        refreshDemoThemes();
        toast.success(`Rolled back to ${theme.manifest.name}`);
      } else {
        if (!client) throw new Error("No store is selected.");
        const theme = await client.themes.rollback();
        reload();
        toast.success(`Rolled back to ${theme.manifest.name}`);
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!deleteTarget || busy) return;
    setBusy("delete");
    try {
      if (mode === "demo") {
        deleteDemoTheme(deleteTarget.id);
        refreshDemoThemes();
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.themes.delete(deleteTarget.id);
        reload();
      }
      toast.success("Theme deleted");
      setDeleteTarget(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  if (!data && mode !== "demo" && loading) return <TableSkeleton />;
  if (!data && mode !== "demo" && error) return <ErrorState message={error} onRetry={reload} />;

  const activeTheme = themes.find((theme) => theme.is_active) ?? null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Themes"
        description="Register storefront themes, choose the active version, and open the editor to shape content."
        actions={
          <>
            <Button variant="outline" onClick={() => void rollback()} disabled={busy !== null}>
              <RotateCcw className="size-4" aria-hidden="true" />
              Roll back
            </Button>
            <Button onClick={openRegister} disabled={busy !== null}>
              <Plus className="size-4" aria-hidden="true" />
              Register theme
            </Button>
          </>
        }
      />

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Registered themes</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              {themes.length} {themes.length === 1 ? "theme" : "themes"}
              {activeTheme ? ` · ${activeTheme.manifest.name} is live` : ""}
            </p>
          </div>
        </div>

        {themes.length === 0 ? (
          <EmptyState
            title="No themes registered"
            description="Register a theme bundle to start authoring pages in the structured editor."
            action={
              <Button onClick={openRegister}>
                <Plus className="size-4" aria-hidden="true" />
                Register theme
              </Button>
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-6">Theme</TableHead>
                <TableHead className="w-[120px]">Status</TableHead>
                <TableHead className="hidden w-[130px] sm:table-cell">Runtime</TableHead>
                <TableHead className="hidden w-[140px] md:table-cell">Updated</TableHead>
                <TableHead className="w-10 pr-5" aria-label="Actions" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {themes.map((theme) => (
                <TableRow
                  key={theme.id}
                  className="h-[58px] cursor-pointer hover:bg-accent/40"
                  onClick={() =>
                    void navigate({ to: "/themes/$themeId", params: { themeId: theme.id } })
                  }
                >
                  <TableCell className="pl-6">
                    <div className="flex items-center gap-3">
                      <span className="flex size-8 items-center justify-center rounded-lg bg-brand-soft text-brand-ink">
                        <History className="size-4" aria-hidden="true" />
                      </span>
                      <div className="min-w-0">
                        <p className="truncate text-[12.5px] font-medium text-foreground">
                          {theme.manifest.name}
                        </p>
                        <p className="mt-0.5 truncate font-mono text-[10.5px] text-faint">
                          {theme.manifest.id} · v{theme.manifest.version}
                        </p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <ThemeActivePill active={theme.is_active} />
                  </TableCell>
                  <TableCell className="hidden text-[11.5px] text-muted-foreground sm:table-cell">
                    <span className="font-mono">{theme.manifest.runtime}</span>
                  </TableCell>
                  <TableCell className="hidden text-[11.5px] text-muted-foreground md:table-cell">
                    {shortDate(fromUnixSeconds(theme.updated_at))}
                  </TableCell>
                  <TableCell
                    className="pr-5 text-right"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Actions for ${theme.manifest.name}`}
                          className="text-faint data-[state=open]:bg-accent"
                        >
                          <MoreHorizontal className="size-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-[200px]">
                        <DropdownMenuLabel>{theme.manifest.name}</DropdownMenuLabel>
                        <DropdownMenuItem
                          onSelect={() =>
                            void navigate({
                              to: "/themes/$themeId",
                              params: { themeId: theme.id },
                            })
                          }
                        >
                          <ArrowUpRight aria-hidden="true" />
                          Open theme
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          disabled={theme.is_active || busy !== null}
                          onSelect={() => void activate(theme)}
                        >
                          <History aria-hidden="true" />
                          {theme.is_active ? "Already active" : "Activate theme"}
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          variant="destructive"
                          disabled={theme.is_active || busy !== null}
                          onSelect={() => setDeleteTarget(theme)}
                        >
                          <Trash2 aria-hidden="true" />
                          Delete theme
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </Card>

      <Dialog open={registerOpen} onOpenChange={setRegisterOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Register theme</DialogTitle>
            <DialogDescription>
              Point Fastcart at a trusted theme bundle. The editor manifest describes the components
              and props page authors can use.
            </DialogDescription>
          </DialogHeader>

          <Tabs
            value={manifestMode}
            onValueChange={(value) => setManifestMode(value as "fields" | "json")}
          >
            <TabsList>
              <TabsTrigger value="fields">Manifest fields</TabsTrigger>
              <TabsTrigger value="json">Paste manifest JSON</TabsTrigger>
            </TabsList>

            <TabsContent value="fields" className="mt-3 space-y-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <Label htmlFor="theme-id">Theme id</Label>
                  <Input
                    id="theme-id"
                    value={form.id}
                    onChange={(event) => setForm({ ...form, id: event.target.value })}
                    placeholder="atelier"
                    className="h-9 font-mono text-[12px]"
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="theme-name">Name</Label>
                  <Input
                    id="theme-name"
                    value={form.name}
                    onChange={(event) => setForm({ ...form, name: event.target.value })}
                    placeholder="Atelier"
                    className="h-9"
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="theme-version">Version</Label>
                  <Input
                    id="theme-version"
                    value={form.version}
                    onChange={(event) => setForm({ ...form, version: event.target.value })}
                    placeholder="1.0.0"
                    className="h-9 font-mono text-[12px]"
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="theme-runtime">Runtime</Label>
                  <Input
                    id="theme-runtime"
                    value={form.runtime}
                    onChange={(event) => setForm({ ...form, runtime: event.target.value })}
                    placeholder="v8"
                    className="h-9 font-mono text-[12px]"
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="theme-entry">Entry</Label>
                  <Input
                    id="theme-entry"
                    value={form.entry}
                    onChange={(event) => setForm({ ...form, entry: event.target.value })}
                    placeholder="dist/server/server.js"
                    className="h-9 font-mono text-[12px]"
                  />
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="theme-bundle">Bundle path</Label>
                  <Input
                    id="theme-bundle"
                    value={form.bundlePath}
                    onChange={(event) => setForm({ ...form, bundlePath: event.target.value })}
                    placeholder="atelier (relative to themes dir)"
                    className="h-9 font-mono text-[12px]"
                  />
                </div>
              </div>
            </TabsContent>

            <TabsContent value="json" className="mt-3">
              <div className="space-y-1.5">
                <Label htmlFor="theme-manifest-json">Manifest JSON</Label>
                <textarea
                  id="theme-manifest-json"
                  value={form.manifestJson}
                  onChange={(event) => setForm({ ...form, manifestJson: event.target.value })}
                  placeholder={
                    '{\n  "id": "atelier",\n  "name": "Atelier",\n  "version": "1.0.0",\n  "runtime": "v8",\n  "entry": "dist/server.js"\n}'
                  }
                  spellCheck={false}
                  className={textareaClass}
                />
              </div>
            </TabsContent>
          </Tabs>

          <div className="grid gap-3 lg:grid-cols-3">
            <div className="space-y-1.5">
              <Label htmlFor="theme-settings-schema">Settings schema JSON</Label>
              <textarea
                id="theme-settings-schema"
                value={form.settingsSchema}
                onChange={(event) => setForm({ ...form, settingsSchema: event.target.value })}
                placeholder={'{\n  "type": "object",\n  "properties": {}\n}'}
                spellCheck={false}
                className={textareaClass}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="theme-settings">Initial settings JSON</Label>
              <textarea
                id="theme-settings"
                value={form.settings}
                onChange={(event) => setForm({ ...form, settings: event.target.value })}
                placeholder={'{\n  "logo": "/logo.svg"\n}'}
                spellCheck={false}
                className={textareaClass}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="theme-editor-manifest">Editor manifest JSON</Label>
              <textarea
                id="theme-editor-manifest"
                value={form.editorManifest}
                onChange={(event) => setForm({ ...form, editorManifest: event.target.value })}
                placeholder={'{\n  "schema_version": 1,\n  "components": {}\n}'}
                spellCheck={false}
                className={textareaClass}
              />
            </div>
          </div>

          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setRegisterOpen(false)}
              disabled={busy !== null}
            >
              Cancel
            </Button>
            <Button onClick={() => void register()} disabled={busy !== null}>
              {busy === "register" ? "Registering…" : "Register theme"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {deleteTarget?.manifest.name}?</DialogTitle>
            <DialogDescription>
              This removes the theme registration. The active theme cannot be deleted, and pages
              authored against it are kept.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDeleteTarget(null)}
              disabled={busy !== null}
            >
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
