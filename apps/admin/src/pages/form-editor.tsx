import { useCallback, useEffect, useMemo, useState } from "react";
import { ArrowLeft, Check, History, Loader2, Save, Send } from "lucide-react";
import { Link, useParams } from "@tanstack/react-router";
import { toast } from "sonner";
import type {
  InquiryFieldKind,
  InquiryForm,
  InquiryFormField,
  InquiryFormRevision,
  InquiryFormSettings,
} from "@fastcart/shared-types";

import { FieldList } from "@/components/inquiry-forms/field-list";
import { FieldPalette } from "@/components/inquiry-forms/field-palette";
import { FieldSettings } from "@/components/inquiry-forms/field-settings";
import { FormPreview } from "@/components/inquiry-forms/form-preview";
import { FormSettingsPanel } from "@/components/inquiry-forms/form-settings-panel";
import { PageHeader } from "@/components/page-header";
import { ErrorState, PageSkeleton } from "@/components/states";
import { InquiryFormStatusPill } from "@/components/status-pill";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
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
import { demoInquiryForm, demoInquiryFormRevisions } from "@/lib/demo-inquiries";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  createField,
  defaultFormSettings,
  draftFingerprint,
  revisionForEditing,
  validateInquiryDraft,
} from "@/lib/inquiry-forms";
import {
  loadInquiryFormEditor,
  publishInquiryForm,
  publishInquiryFormVersion,
  saveInquiryFormDraft,
} from "@/lib/loaders";
import { useAsyncData } from "@/lib/use-async-data";

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function FormEditorPage() {
  const { formId } = useParams({ from: "/forms/$formId" });
  const { mode, client, store, dataKey } = useAuth();
  const [fields, setFields] = useState<InquiryFormField[]>([]);
  const [settings, setSettings] = useState<InquiryFormSettings>(defaultFormSettings);
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [baseline, setBaseline] = useState("");
  const [tab, setTab] = useState<string>("build");
  const [errors, setErrors] = useState<string[]>([]);
  const [busy, setBusy] = useState<"save" | "publish" | "version" | null>(null);
  const [revisionsOpen, setRevisionsOpen] = useState(false);
  const [localForm, setLocalForm] = useState<InquiryForm | null>(null);
  const [demoRevisions, setDemoRevisions] = useState<InquiryFormRevision[] | null>(null);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") {
        const form = demoInquiryForm(formId);
        if (!form) return Promise.reject(new Error("This form could not be found."));
        return Promise.resolve({
          form,
          revisions: demoInquiryFormRevisions(formId),
        });
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadInquiryFormEditor(client, formId, signal);
    },
    [mode, client, formId],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    formId,
  ]);

  const revisions = useMemo(() => {
    if (mode === "demo" && demoRevisions !== null) return demoRevisions;
    return data?.revisions ?? [];
  }, [mode, demoRevisions, data]);

  const form = localForm ?? data?.form ?? null;

  useEffect(() => {
    if (!data) return;
    setLocalForm(data.form);
    if (mode === "demo" && demoRevisions === null) setDemoRevisions(data.revisions);
    const revision = revisionForEditing(data.revisions);
    const nextFields = revision ? clone(revision.fields) : [];
    const nextSettings = revision ? clone(revision.settings) : defaultFormSettings();
    setFields(nextFields);
    setSettings(nextSettings);
    setBaseline(draftFingerprint(nextFields, nextSettings));
    setSelectedIndex(nextFields.length > 0 ? 0 : null);
    setErrors([]);
    // Only re-initialize when fresh server data arrives.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  const fingerprint = draftFingerprint(fields, settings);
  const dirty = baseline.length > 0 && fingerprint !== baseline;

  useEffect(() => {
    if (!dirty) return;
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", onBeforeUnload);
    return () => window.removeEventListener("beforeunload", onBeforeUnload);
  }, [dirty]);

  const selectedField = selectedIndex === null ? null : (fields[selectedIndex] ?? null);

  const addField = (kind: InquiryFieldKind) => {
    const field = createField(kind, fields);
    setFields((current) => [...current, field]);
    setSelectedIndex(fields.length);
  };

  const moveField = (index: number, direction: -1 | 1) => {
    setFields((current) => {
      const target = index + direction;
      if (target < 0 || target >= current.length) return current;
      const next = [...current];
      const entry = next[index];
      const swap = next[target];
      if (!entry || !swap) return current;
      next[index] = swap;
      next[target] = entry;
      return next;
    });
    setSelectedIndex((current) => {
      if (current === null) return current;
      const target = index + direction;
      if (current === index) return target;
      if (current === target) return index;
      return current;
    });
  };

  const deleteField = (index: number) => {
    const removed = fields[index];
    setFields((current) => current.filter((_, entry) => entry !== index));
    setSettings((current) =>
      removed
        ? {
            ...current,
            notification_fields: current.notification_fields.filter(
              (key) => key !== removed.key,
            ),
          }
        : current,
    );
    setSelectedIndex((current) => {
      if (current === null) return null;
      if (current === index) return fields.length > 1 ? Math.min(index, fields.length - 2) : null;
      return current > index ? current - 1 : current;
    });
  };

  const updateField = (next: InquiryFormField) => {
    if (selectedIndex === null) return;
    setFields((current) =>
      current.map((field, index) => (index === selectedIndex ? next : field)),
    );
  };

  const runValidation = (): boolean => {
    const nextErrors = validateInquiryDraft(fields, settings);
    setErrors(nextErrors);
    if (nextErrors.length > 0) {
      toast.error(nextErrors[0] ?? "Fix the highlighted fields before continuing.");
      return false;
    }
    return true;
  };

  const saveDraft = async (): Promise<boolean> => {
    if (busy) return false;
    if (!runValidation()) return false;
    setBusy("save");
    try {
      if (mode === "demo") {
        setBaseline(fingerprint);
        toast.success("Draft saved in the demo");
      } else {
        if (!client) throw new Error("No store is selected.");
        await saveInquiryFormDraft(client, formId, fields, settings);
        setBaseline(fingerprint);
        reload();
        toast.success("Draft saved");
      }
      return true;
    } catch (cause) {
      toast.error(errorMessage(cause));
      return false;
    } finally {
      setBusy(null);
    }
  };

  const publish = async () => {
    if (busy) return;
    if (!runValidation()) return;
    setBusy("publish");
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        const version = localForm?.current_version ?? 1;
        setLocalForm((current) =>
          current
            ? { ...current, status: "published", published_version: version, updated_at: now }
            : current,
        );
        setDemoRevisions((current) =>
          (current ?? []).map((revision) =>
            revision.published_at === null || revision.published_at === undefined
              ? { ...revision, published_at: now }
              : revision,
          ),
        );
        setBaseline(fingerprint);
        toast.success(`Published version ${version}`);
      } else {
        if (!client) throw new Error("No store is selected.");
        await saveInquiryFormDraft(client, formId, fields, settings);
        const form = await publishInquiryForm(client, formId);
        setBaseline(fingerprint);
        reload();
        toast.success(`Published version ${form.published_version ?? "draft"}`);
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const publishVersion = async (version: number) => {
    if (busy) return;
    setBusy("version");
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        setLocalForm((current) =>
          current
            ? { ...current, status: "published", published_version: version, updated_at: now }
            : current,
        );
        setDemoRevisions((current) =>
          (current ?? []).map((revision) =>
            revision.version === version ? { ...revision, published_at: now } : revision,
          ),
        );
        toast.success(`Published version ${version}`);
      } else {
        if (!client) throw new Error("No store is selected.");
        await publishInquiryFormVersion(client, formId, version);
        reload();
        toast.success(`Published version ${version}`);
      }
      setRevisionsOpen(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  if (!data && loading) return <PageSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!form) return null;

  const draftVersion = revisionForEditing(revisions);

  return (
    <div className="space-y-6 animate-enter">
      <div>
        <Link
          to="/forms"
          className="inline-flex items-center gap-1.5 text-[11.5px] text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          All forms
        </Link>
      </div>

      <PageHeader
        title={form.name}
        description={`/${form.handle}${form.description ? ` · ${form.description}` : ""}`}
        actions={
          <>
            <div className="mr-1 hidden items-center gap-2 md:flex">
              <InquiryFormStatusPill status={form.status} />
              {dirty ? <Badge variant="warning">Unsaved changes</Badge> : null}
            </div>
            <Button variant="outline" onClick={() => setRevisionsOpen(true)}>
              <History className="size-4" aria-hidden="true" />
              Revisions
            </Button>
            <Button
              variant="outline"
              onClick={() => void saveDraft()}
              disabled={busy !== null}
            >
              {busy === "save" ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Save className="size-4" aria-hidden="true" />
              )}
              Save draft
            </Button>
            <Button onClick={() => void publish()} disabled={busy !== null}>
              {busy === "publish" ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Send className="size-4" aria-hidden="true" />
              )}
              Publish
            </Button>
          </>
        }
      />

      {errors.length > 0 ? (
        <Card className="border-destructive/25 bg-destructive/5 p-4">
          <p className="text-[12.5px] font-medium text-destructive">
            Fix these before saving or publishing
          </p>
          <ul className="mt-2 list-disc space-y-1 pl-5 text-[11.5px] text-destructive/90">
            {errors.slice(0, 8).map((message) => (
              <li key={message}>{message}</li>
            ))}
          </ul>
        </Card>
      ) : null}

      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="build">Build</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="build" className="mt-4">
          <div className="grid items-start gap-4 xl:grid-cols-[164px_minmax(0,1fr)_minmax(260px,300px)]">
            <Card className="p-2.5 xl:sticky xl:top-[88px]">
              <FieldPalette onAdd={addField} />
            </Card>

            <div className="min-w-0 space-y-4">
              <Card className="p-4">
                <div className="mb-3 flex items-center justify-between gap-2">
                  <div>
                    <p className="text-[12.5px] font-medium text-foreground">Fields</p>
                    <p className="mt-0.5 text-[10.5px] text-faint">
                      {fields.length} {fields.length === 1 ? "field" : "fields"} · use the arrows to
                      reorder
                    </p>
                  </div>
                  {draftVersion ? (
                    <span className="font-mono text-[10.5px] text-faint">
                      draft v{draftVersion.version}
                    </span>
                  ) : null}
                </div>
                <FieldList
                  fields={fields}
                  selectedIndex={selectedIndex}
                  onSelect={setSelectedIndex}
                  onMove={moveField}
                  onDelete={deleteField}
                />
              </Card>

              <FormPreview fields={fields} settings={settings} />
            </div>

            <Card className="p-4 xl:sticky xl:top-[88px]">
              <FieldSettings
                field={selectedField}
                onChange={updateField}
                onDelete={() => selectedIndex !== null && deleteField(selectedIndex)}
              />
            </Card>
          </div>
        </TabsContent>

        <TabsContent value="settings" className="mt-4">
          <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
            <Card className="p-5">
              <FormSettingsPanel
                settings={settings}
                fields={fields}
                onChange={setSettings}
              />
            </Card>
            <Card className="p-5">
              <FormPreview fields={fields} settings={settings} />
            </Card>
          </div>
        </TabsContent>
      </Tabs>

      <Sheet open={revisionsOpen} onOpenChange={setRevisionsOpen}>
        <SheetContent side="right" className="sm:max-w-md">
          <SheetHeader>
            <SheetTitle>Revisions</SheetTitle>
            <SheetDescription>
              Publishing a revision makes it live. Older revisions stay available for rollback.
            </SheetDescription>
          </SheetHeader>
          <div className="flex-1 space-y-2 overflow-y-auto p-6">
            {revisions.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12px] text-muted-foreground">
                No revisions yet.
              </p>
            ) : (
              revisions.map((revision) => {
                const published = revision.published_at !== null && revision.published_at !== undefined;
                const isLive = form.published_version === revision.version;
                return (
                  <div
                    key={revision.id}
                    className="rounded-xl border border-border bg-card p-3.5"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-[12px] font-medium text-foreground">
                          v{revision.version}
                        </span>
                        {isLive ? (
                          <Badge variant="brand">Live</Badge>
                        ) : published ? (
                          <Badge variant="secondary">Published</Badge>
                        ) : (
                          <Badge variant="warning">Draft</Badge>
                        )}
                      </div>
                      <span className="text-[10.5px] text-faint">
                        {shortDate(fromUnixSeconds(revision.created_at))}
                      </span>
                    </div>
                    <p className="mt-1.5 text-[11px] text-muted-foreground">
                      {revision.fields.length} fields ·{" "}
                      {published
                        ? `published ${shortDate(fromUnixSeconds(revision.published_at ?? revision.created_at))}`
                        : "not published"}
                    </p>
                    <div className="mt-3 flex items-center justify-end gap-2">
                      {published ? (
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={busy !== null || isLive}
                          onClick={() => void publishVersion(revision.version)}
                        >
                          <Check className="size-3.5" aria-hidden="true" />
                          {isLive ? "Current" : "Roll back"}
                        </Button>
                      ) : (
                        <Button
                          size="sm"
                          disabled={busy !== null}
                          onClick={() => {
                            setRevisionsOpen(false);
                            void publish();
                          }}
                        >
                          <Send className="size-3.5" aria-hidden="true" />
                          Publish draft
                        </Button>
                      )}
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </SheetContent>
      </Sheet>
    </div>
  );
}
