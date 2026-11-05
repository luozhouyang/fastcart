import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useParams } from "@tanstack/react-router";
import {
  AlertTriangle,
  ArrowLeft,
  Eye,
  History,
  Layers3,
  Loader2,
  RotateCcw,
  Save,
  Send,
  Undo2,
} from "lucide-react";
import { toast } from "sonner";
import { ApiError } from "@fastcart/api-client";
import type {
  JsonValue,
  Page,
  PageBlock,
  PageBody,
  PageVersion,
  PreviewTokenResponse,
  Theme,
  ThemeEditorManifest,
} from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { ErrorState, PageSkeleton } from "@/components/states";
import { PageStatusPill } from "@/components/status-pill";
import { ComponentPalette } from "@/components/theme-editor/component-palette";
import { BlockTree } from "@/components/theme-editor/block-tree";
import { BlockFields } from "@/components/theme-editor/field-editor";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
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
import { useAuth } from "@/lib/auth";
import { API_BASE_URL, errorMessage } from "@/lib/api";
import {
  getDemoActiveTheme,
  getDemoEditorManifest,
  getDemoPage,
  issueDemoPreviewToken,
  listDefaultPageVersions,
  publishDemoPage,
  restoreDemoPageVersion,
  unpublishDemoPage,
  updateDemoPage,
} from "@/lib/demo-content";
import { fromUnixSeconds, shortDate, timeOfDay } from "@/lib/format";
import {
  appendBlock,
  appendBlockToSlot,
  blockId,
  blockProps,
  buildBlock,
  collectBlockIds,
  componentTitle,
  findBlock,
  isJsonObject,
  moveBlock,
  normalizePageBody,
  pageThemeRef,
  parseJsonText,
  prettyJson,
  removeBlock,
  updateBlockProps,
} from "@/lib/theme-editor";
import { useAsyncData } from "@/lib/use-async-data";

const pendingBodyClass =
  "min-h-[520px] w-full resize-y rounded-md border border-input bg-card px-3 py-3 font-mono text-[11.5px] leading-relaxed text-foreground shadow-xs outline-none transition-[color,box-shadow,border-color] placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25";

interface PageEditorData {
  page: Page;
  activeTheme: Theme | null;
  editorManifest: ThemeEditorManifest | null;
  versions: PageVersion[];
}

function pageBodyToJson(body: PageBody): JsonValue {
  return JSON.parse(JSON.stringify(body)) as JsonValue;
}

function isConflict(cause: unknown): boolean {
  return cause instanceof ApiError && cause.status === 409;
}

/** Finds a block in page content or any zone. */
function findBlockInBody(
  content: PageBlock[],
  zones: Array<[string, PageBlock[]]>,
  id: string,
): PageBlock | null {
  const direct = findBlock(content, id);
  if (direct) return direct;
  for (const [, blocks] of zones) {
    const found = findBlock(blocks, id);
    if (found) return found;
  }
  return null;
}

/** Returns the owner (`content` or a zone name) that contains a block. */
function ownerOf(content: PageBlock[], zones: Array<[string, PageBlock[]]>, id: string): string {
  if (findBlock(content, id)) return "content";
  for (const [name, blocks] of zones) {
    if (findBlock(blocks, id)) return name;
  }
  return "content";
}

export function PageEditorPage() {
  const { pageId } = useParams({ from: "/pages/$pageId" });
  const { mode, client, store, dataKey } = useAuth();
  const [body, setBody] = useState<PageBody>(normalizePageBody(null));
  const [baseline, setBaseline] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [target, setTarget] = useState("content");
  const [jsonText, setJsonText] = useState("");
  const [jsonBaseline, setJsonBaseline] = useState("");
  const [busy, setBusy] = useState<"save" | "publish" | "unpublish" | "preview" | "restore" | null>(
    null,
  );
  const [versionsOpen, setVersionsOpen] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [previewOrigin, setPreviewOrigin] = useState<string | null>(null);
  const [previewToken, setPreviewToken] = useState<PreviewTokenResponse | null>(null);
  const [previewHandle, setPreviewHandle] = useState("");
  const [previewHandleDraft, setPreviewHandleDraft] = useState("");

  const loader = useCallback(
    async (signal: AbortSignal): Promise<PageEditorData> => {
      if (mode === "demo") {
        const page = getDemoPage(pageId);
        if (!page) throw new Error("This page could not be found.");
        const activeTheme = getDemoActiveTheme();
        return {
          page,
          activeTheme,
          editorManifest: activeTheme ? getDemoEditorManifest(activeTheme.id) : null,
          versions: listDefaultPageVersions(pageId),
        };
      }
      if (!client) throw new Error("No store is selected.");
      const page = await client.content.pages.get(pageId, { signal });
      const activeTheme = await client.themes.active({ signal }).catch(() => null);
      let editorManifest: ThemeEditorManifest | null = null;
      if (activeTheme && isJsonObject(activeTheme.editor_manifest)) {
        editorManifest = activeTheme.editor_manifest as unknown as ThemeEditorManifest;
      } else if (activeTheme) {
        try {
          editorManifest = await client.themes.editorManifest(activeTheme.id, { signal });
        } catch {
          editorManifest = null;
        }
      }
      const versions = await client.content.pages
        .versions(pageId, { limit: 50 }, { signal })
        .then((page) => page.data)
        .catch(() => []);
      return { page, activeTheme, editorManifest, versions };
    },
    [mode, client, pageId],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey, pageId]);

  const editorManifest = data?.editorManifest ?? null;
  const structured =
    editorManifest !== null && Object.keys(editorManifest.components ?? {}).length > 0;

  useEffect(() => {
    if (!data) return;
    const normalized = normalizePageBody(data.page.body);
    setBody(normalized);
    setBaseline(JSON.stringify(normalized));
    setSelectedId(null);
    setJsonText(prettyJson(data.page.body));
    setJsonBaseline(prettyJson(data.page.body));
  }, [data]);

  const content = body.content ?? [];
  const zones = useMemo(() => Object.entries(body.zones ?? {}), [body.zones]);
  const selectedBlock = selectedId ? findBlockInBody(content, zones, selectedId) : null;
  const parsedJson = useMemo(() => parseJsonText(jsonText), [jsonText]);

  const dirty = structured ? JSON.stringify(body) !== baseline : jsonText !== jsonBaseline;

  useEffect(() => {
    if (!dirty) return;
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, [dirty]);

  const page = data?.page ?? null;
  const previewRoutePattern = data?.activeTheme?.manifest.routes?.[page?.page_type ?? ""] ?? null;
  const previewSrc =
    previewOrigin && previewToken && page
      ? `${previewOrigin}/preview${previewPathFor(previewHandle, page.handle, previewRoutePattern)}?token=${encodeURIComponent(previewToken.token)}`
      : null;

  /** Replaces the block list of one owner (`content` or a zone name). */
  const updateBlocks = (owner: string, update: (blocks: PageBlock[]) => PageBlock[]) => {
    setBody((current) => {
      if (owner === "content") {
        return { ...current, content: update(current.content ?? []) };
      }
      const nextZones = { ...(current.zones ?? {}) };
      nextZones[owner] = update(nextZones[owner] ?? []);
      return { ...current, zones: nextZones };
    });
  };

  /** Block identifiers are unique across content and every zone. */
  const allBlockIds = (): Set<string> => {
    const ids = collectBlockIds(content);
    for (const [, blocks] of zones) {
      collectBlockIds(blocks, ids);
    }
    return ids;
  };

  const addBlock = (type: string) => {
    const owner = target;
    const block = buildBlock(type, editorManifest, allBlockIds());
    updateBlocks(owner, (blocks) => appendBlock(blocks, block));
    const id = blockId(block);
    if (id) setSelectedId(id);
  };

  const addBlockToSlot = (parentId: string, slotName: string, type: string) => {
    const owner = ownerOf(content, zones, parentId);
    const block = buildBlock(type, editorManifest, allBlockIds());
    updateBlocks(owner, (blocks) => appendBlockToSlot(blocks, parentId, slotName, block));
    const id = blockId(block);
    if (id) setSelectedId(id);
  };

  const moveSelected = (id: string, direction: -1 | 1) => {
    const owner = ownerOf(content, zones, id);
    updateBlocks(owner, (blocks) => moveBlock(blocks, id, direction));
  };

  const removeSelected = (id: string) => {
    const owner = ownerOf(content, zones, id);
    updateBlocks(owner, (blocks) => removeBlock(blocks, id));
    if (selectedId === id) setSelectedId(null);
  };

  const declaredZones = editorManifest?.zones ?? {};
  const availableZones = useMemo(
    () =>
      Object.entries(editorManifest?.zones ?? {}).filter(([name]) => !(name in (body.zones ?? {}))),
    [editorManifest, body.zones],
  );
  const addZone = (name: string) => {
    setBody((current) => ({
      ...current,
      zones: { ...(current.zones ?? {}), [name]: [] },
    }));
  };
  const removeZone = (name: string) => {
    setBody((current) => {
      const next = { ...(current.zones ?? {}) };
      delete next[name];
      return { ...current, zones: next };
    });
    if (target === name) setTarget("content");
  };

  const changeProp = (key: string, value: JsonValue | undefined) => {
    if (!selectedId) return;
    const owner = ownerOf(content, zones, selectedId);
    updateBlocks(owner, (blocks) =>
      updateBlockProps(blocks, selectedId, (props) => {
        const updated = { ...props };
        if (value === undefined) delete updated[key];
        else updated[key] = value;
        return updated;
      }),
    );
  };

  const save = async () => {
    if (!page || busy) return;
    let bodyJson: JsonValue;
    if (structured) {
      const withTheme: PageBody = {
        ...body,
        schema_version: body.schema_version ?? 1,
        theme: pageThemeRef(data?.activeTheme ?? null) ?? body.theme ?? null,
        content,
        root: body.root ?? { props: {} },
        zones: body.zones ?? {},
      };
      bodyJson = pageBodyToJson(withTheme);
    } else {
      const parsed = parseJsonText(jsonText);
      if (!parsed.ok) {
        toast.error(`Page body is not valid JSON: ${parsed.error ?? "invalid JSON"}`);
        return;
      }
      bodyJson = parsed.value ?? {};
    }

    setBusy("save");
    try {
      if (mode === "demo") {
        updateDemoPage(pageId, bodyJson, page.revision);
        toast.success("Page saved in the demo");
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.content.pages.update(pageId, {
          base_revision: page.revision,
          body: bodyJson,
        });
        toast.success("Page saved");
      }
      reload();
    } catch (cause) {
      if (isConflict(cause)) {
        toast.error("This page changed elsewhere. Reload the page before saving.");
      } else {
        toast.error(errorMessage(cause));
      }
    } finally {
      setBusy(null);
    }
  };

  const publish = async () => {
    if (!page || busy) return;
    if (dirty) {
      toast.error("Save your changes before publishing.");
      return;
    }
    setBusy("publish");
    try {
      if (mode === "demo") {
        publishDemoPage(pageId);
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.content.pages.publish(pageId);
      }
      reload();
      toast.success("Page published");
    } catch (cause) {
      if (isConflict(cause)) {
        toast.error("This page changed elsewhere. Reload before publishing.");
      } else {
        toast.error(errorMessage(cause));
      }
    } finally {
      setBusy(null);
    }
  };

  const unpublish = async () => {
    if (!page || busy) return;
    setBusy("unpublish");
    try {
      if (mode === "demo") {
        unpublishDemoPage(pageId);
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.content.pages.unpublish(pageId);
      }
      reload();
      toast.success("Page unpublished");
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const openPreview = async () => {
    if (!page || busy) return;
    if (dirty) {
      toast.error("Save your changes to preview them.");
      return;
    }
    setBusy("preview");
    try {
      let token: PreviewTokenResponse;
      if (mode === "demo") {
        token = issueDemoPreviewToken(pageId);
      } else {
        if (!client) throw new Error("No store is selected.");
        token = await client.content.pages.previewToken(pageId);
      }
      // Render the preview from the storefront origin so relative theme assets
      // and storefront API calls resolve against the store rather than the
      // admin host. Fall back to the API origin when no domain is configured.
      const origin = store?.primary_domain
        ? `${window.location.protocol}//${store.primary_domain}`
        : API_BASE_URL;
      setPreviewToken(token);
      setPreviewOrigin(origin);
      setPreviewHandleDraft(previewHandle);
      setPreviewOpen(true);
    } catch (cause) {
      toast.error(
        cause instanceof ApiError && cause.status === 501
          ? "Draft preview is not configured on this server."
          : errorMessage(cause),
      );
    } finally {
      setBusy(null);
    }
  };

  const closePreview = (open: boolean) => {
    if (open) {
      setPreviewOpen(true);
      return;
    }
    setPreviewOpen(false);
    setPreviewOrigin(null);
    const token = previewToken;
    setPreviewToken(null);
    if (token && mode !== "demo" && client) {
      void client.content.pages.revoke(pageId, { jti: token.jti }).catch(() => {
        // Revocation is best effort; the token expires shortly anyway.
      });
    }
  };

  const restoreVersion = async (version: PageVersion) => {
    if (!page || busy) return;
    setBusy("restore");
    try {
      if (mode === "demo") {
        restoreDemoPageVersion(pageId, version.version);
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.content.pages.restore(pageId, version.version, {
          base_revision: page.revision,
        });
      }
      setVersionsOpen(false);
      reload();
      toast.success(`Restored version ${version.version}`);
    } catch (cause) {
      if (isConflict(cause)) {
        toast.error("This page changed elsewhere. Reload before restoring a version.");
      } else {
        toast.error(errorMessage(cause));
      }
    } finally {
      setBusy(null);
    }
  };

  if (!data && loading) return <PageSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;
  if (!page) return null;

  const props = selectedBlock ? blockProps(selectedBlock) : {};
  const selectedComponent = selectedBlock
    ? (editorManifest?.components?.[selectedBlock.type] ?? null)
    : null;

  return (
    <div className="space-y-6 animate-enter">
      <div>
        <Link
          to="/pages"
          className="inline-flex items-center gap-1.5 text-[11.5px] text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          All pages
        </Link>
      </div>

      <PageHeader
        title={page.title}
        description={`/${page.handle} · ${page.page_type} · ${structured ? "structured editor" : "JSON body editor"}`}
        actions={
          <>
            <div className="mr-1 hidden items-center gap-2 md:flex">
              <PageStatusPill status={page.status} />
              {dirty ? <Badge variant="warning">Unsaved changes</Badge> : null}
            </div>
            <Button variant="outline" onClick={() => setVersionsOpen(true)}>
              <History className="size-4" aria-hidden="true" />
              Versions
            </Button>
            <Button variant="outline" onClick={() => void openPreview()} disabled={busy !== null}>
              {busy === "preview" ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Eye className="size-4" aria-hidden="true" />
              )}
              Preview
            </Button>
            <Button
              variant="outline"
              onClick={() => void save()}
              disabled={busy !== null || !dirty}
            >
              {busy === "save" ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Save className="size-4" aria-hidden="true" />
              )}
              Save
            </Button>
            {page.status === "published" ? (
              <Button variant="outline" onClick={() => void unpublish()} disabled={busy !== null}>
                <Undo2 className="size-4" aria-hidden="true" />
                Unpublish
              </Button>
            ) : (
              <Button
                onClick={() => void publish()}
                disabled={busy !== null || page.status === "archived"}
              >
                {busy === "publish" ? (
                  <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                ) : (
                  <Send className="size-4" aria-hidden="true" />
                )}
                Publish
              </Button>
            )}
          </>
        }
      />

      {!structured ? (
        <Card className="border-warning/25 bg-warning-soft/30 p-4">
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 size-4 shrink-0 text-warning" aria-hidden="true" />
            <div>
              <p className="text-[12.5px] font-medium text-foreground">
                No active theme editor manifest
              </p>
              <p className="mt-0.5 text-[11.5px] text-muted-foreground">
                Activate a theme that ships an editor manifest to use the component palette. You can
                still edit the raw JSON body below.
              </p>
            </div>
          </div>
        </Card>
      ) : null}

      {structured ? (
        <div className="grid items-start gap-4 xl:grid-cols-[180px_minmax(0,1fr)_minmax(260px,320px)]">
          <Card className="p-2.5 xl:sticky xl:top-[88px]">
            <ComponentPalette
              manifest={editorManifest}
              onAdd={addBlock}
              target={target}
              onTargetChange={setTarget}
              targets={[
                { value: "content", label: "Page content" },
                ...zones.map(([name]) => ({
                  value: name,
                  label: declaredZones[name]?.title || name,
                })),
              ]}
            />
          </Card>

          <div className="min-w-0 space-y-4">
            <Card className="p-4">
              <div className="mb-3 flex items-center justify-between gap-2">
                <div>
                  <p className="text-[12.5px] font-medium text-foreground">Page blocks</p>
                  <p className="mt-0.5 text-[10.5px] text-faint">
                    {content.length} top-level {content.length === 1 ? "block" : "blocks"} · nested
                    slots are editable inline
                  </p>
                </div>
                <span className="font-mono text-[10.5px] text-faint">r{page.revision}</span>
              </div>
              <BlockTree
                blocks={content}
                manifest={editorManifest}
                selectedId={selectedId}
                onSelect={setSelectedId}
                onMove={moveSelected}
                onRemove={removeSelected}
                onAddToSlot={addBlockToSlot}
              />
            </Card>

            {zones.map(([name, blocks]) => {
              const declared = Object.hasOwn(declaredZones, name);
              return (
                <Card key={name} className="p-4">
                  <div className="mb-3 flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <p className="truncate text-[12.5px] font-medium text-foreground">
                        Zone · {declared ? declaredZones[name]?.title : name}
                      </p>
                      <p className="mt-0.5 truncate font-mono text-[10px] text-faint">
                        {name} · {blocks.length} {blocks.length === 1 ? "block" : "blocks"}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-1.5">
                      {declared ? null : (
                        <Badge variant="destructive" className="text-[10px]">
                          Not in theme
                        </Badge>
                      )}
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="text-faint hover:text-destructive"
                        aria-label={`Remove zone ${name}`}
                        onClick={() => removeZone(name)}
                      >
                        <Undo2 className="size-3.5" />
                      </Button>
                    </div>
                  </div>
                  <BlockTree
                    blocks={blocks}
                    manifest={editorManifest}
                    selectedId={selectedId}
                    onSelect={setSelectedId}
                    onMove={moveSelected}
                    onRemove={removeSelected}
                    onAddToSlot={addBlockToSlot}
                  />
                </Card>
              );
            })}

            {availableZones.length > 0 ? (
              <Card className="p-4">
                <p className="text-[12.5px] font-medium text-foreground">Add a zone</p>
                <p className="mt-0.5 text-[10.5px] text-faint">
                  Zones declared by the active theme. Zones that are not declared block saving and
                  theme activation.
                </p>
                <div className="mt-3 flex flex-wrap gap-1.5">
                  {availableZones.map(([name, zone]) => (
                    <Button key={name} variant="outline" size="sm" onClick={() => addZone(name)}>
                      Add {zone.title || name}
                    </Button>
                  ))}
                </div>
              </Card>
            ) : null}

            <Card className="p-4">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <Layers3 className="size-3.5 text-faint" aria-hidden="true" />
                  <p className="text-[12.5px] font-medium text-foreground">Page body JSON</p>
                </div>
                <span className="text-[10.5px] text-faint">
                  Read-only preview of the saved shape
                </span>
              </div>
              <pre className="mt-2 max-h-[220px] overflow-auto rounded-lg border border-border bg-background p-3 font-mono text-[10.5px] leading-relaxed text-ink-soft">
                {prettyJson(pageBodyToJson(body))}
              </pre>
            </Card>
          </div>

          <Card className="p-4 xl:sticky xl:top-[88px]">
            <div className="mb-3 flex items-center justify-between gap-2">
              <div className="min-w-0">
                <p className="truncate text-[12.5px] font-medium text-foreground">
                  {selectedBlock
                    ? componentTitle(editorManifest, selectedBlock.type)
                    : "Block settings"}
                </p>
                <p className="mt-0.5 truncate font-mono text-[10px] text-faint">
                  {selectedBlock
                    ? `${selectedBlock.type}${selectedId ? ` · ${selectedId}` : ""}`
                    : "Select a block to edit its props"}
                </p>
                {selectedComponent?.description ? (
                  <p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">
                    {selectedComponent.description}
                  </p>
                ) : null}
              </div>
              {selectedBlock && selectedId ? (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-faint hover:text-destructive"
                  aria-label="Delete selected block"
                  onClick={() => removeSelected(selectedId)}
                >
                  <Undo2 className="size-3.5" />
                </Button>
              ) : null}
            </div>
            {selectedBlock ? (
              <BlockFields
                manifest={editorManifest}
                type={selectedBlock.type}
                props={props}
                onChange={changeProp}
                client={mode === "demo" ? null : client}
              />
            ) : (
              <div className="rounded-xl border border-dashed border-border px-4 py-10 text-center">
                <p className="text-[12.5px] font-medium text-foreground">Nothing selected</p>
                <p className="mt-1 text-[11.5px] text-muted-foreground">
                  Pick a block in the tree, or add one from the palette.
                </p>
              </div>
            )}
          </Card>
        </div>
      ) : (
        <Card className="p-4">
          <div className="flex items-center justify-between gap-2">
            <p className="text-[12.5px] font-medium text-foreground">Page body</p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                const parsed = parseJsonText(jsonText);
                if (parsed.ok) {
                  setJsonText(prettyJson(parsed.value ?? null));
                  toast.success("JSON formatted");
                } else {
                  toast.error(parsed.error ?? "Invalid JSON");
                }
              }}
            >
              Format JSON
            </Button>
          </div>
          <textarea
            value={jsonText}
            onChange={(event) => setJsonText(event.target.value)}
            spellCheck={false}
            aria-invalid={!parsedJson.ok}
            className={`${pendingBodyClass} mt-3`}
          />
          {!parsedJson.ok ? (
            <p className="mt-1 flex items-center gap-1.5 text-[10.5px] text-destructive">
              <AlertTriangle className="size-3" aria-hidden="true" />
              Invalid JSON: {parsedJson.error}
            </p>
          ) : null}
        </Card>
      )}

      <Sheet open={versionsOpen} onOpenChange={setVersionsOpen}>
        <SheetContent side="right" className="sm:max-w-md">
          <SheetHeader>
            <SheetTitle>Page versions</SheetTitle>
            <SheetDescription>
              Draft and published snapshots. Restoring writes a new revision from the snapshot.
            </SheetDescription>
          </SheetHeader>
          <div className="flex-1 space-y-2 overflow-y-auto p-6">
            {data.versions.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12px] text-muted-foreground">
                No versions yet.
              </p>
            ) : (
              data.versions.map((version) => (
                <div key={version.id} className="rounded-xl border border-border bg-card p-3.5">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-[12px] font-medium text-foreground">
                        v{version.version}
                      </span>
                      <Badge variant={version.kind === "published" ? "brand" : "warning"}>
                        {version.kind === "published" ? "Published" : "Draft"}
                      </Badge>
                      {version.origin === "ai" ? <Badge variant="info">AI</Badge> : null}
                    </div>
                    <span className="text-[10.5px] text-faint">
                      {shortDate(fromUnixSeconds(version.created_at))}
                    </span>
                  </div>
                  <div className="mt-3 flex justify-end">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={busy !== null || version.version === page.revision}
                      onClick={() => void restoreVersion(version)}
                    >
                      <RotateCcw className="size-3.5" aria-hidden="true" />
                      {version.version === page.revision ? "Current" : "Restore"}
                    </Button>
                  </div>
                </div>
              ))
            )}
          </div>
        </SheetContent>
      </Sheet>

      <Dialog open={previewOpen} onOpenChange={closePreview}>
        <DialogContent className="sm:max-w-5xl">
          <DialogHeader>
            <DialogTitle>Draft preview</DialogTitle>
            <DialogDescription>
              Rendering {page.title} with a short-lived preview token. The token is revoked when
              this dialog closes.
            </DialogDescription>
          </DialogHeader>
          {previewSrc ? (
            <div className="space-y-3">
              {previewRoutePattern ? (
                <form
                  className="flex flex-wrap items-center gap-2"
                  onSubmit={(event) => {
                    event.preventDefault();
                    setPreviewHandle(previewHandleDraft.trim());
                  }}
                >
                  <label htmlFor="preview-handle" className="text-[11.5px] text-muted-foreground">
                    Preview handle
                  </label>
                  <Input
                    id="preview-handle"
                    value={previewHandleDraft}
                    onChange={(event) => setPreviewHandleDraft(event.target.value)}
                    placeholder="first published item"
                    className="h-8 max-w-56"
                  />
                  <Button type="submit" variant="outline" size="sm">
                    Apply
                  </Button>
                  <span className="font-mono text-[10px] text-faint">{previewRoutePattern}</span>
                </form>
              ) : null}
              <iframe
                title={`Preview of ${page.title}`}
                src={previewSrc}
                className="h-[68vh] w-full rounded-lg border border-border bg-white"
                sandbox="allow-scripts allow-same-origin allow-forms"
              />
              <div className="flex items-center justify-between gap-2">
                <p className="font-mono text-[10px] text-faint">
                  expires{" "}
                  {previewToken ? timeOfDay(fromUnixSeconds(previewToken.expires_at)) : "soon"}
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => window.open(previewSrc, "_blank", "noopener,noreferrer")}
                >
                  <Eye className="size-3.5" aria-hidden="true" />
                  Open in new tab
                </Button>
              </div>
            </div>
          ) : (
            <p className="py-8 text-center text-[12px] text-muted-foreground">
              Requesting a preview token…
            </p>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => closePreview(false)}>
              Close preview
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/**
 * Builds the storefront path for a draft preview.
 *
 * Dynamic route templates use their declared pattern when a handle is
 * supplied; otherwise the server auto-selects the first published record and
 * the page handle is kept as a harmless fallback path.
 */
function previewPathFor(handle: string, pageHandle: string, routePattern: string | null): string {
  const trimmed = handle.trim();
  if (routePattern && trimmed) {
    return routePattern.replace("{handle}", encodeURIComponent(trimmed));
  }
  return `/${encodeURIComponent(pageHandle)}`;
}
