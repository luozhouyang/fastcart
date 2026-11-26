import { useEffect, useId, useMemo, useState } from "react";
import { ChevronDown, Languages, Loader2, RotateCcw, Save, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { StoreLocale, Translation, TranslationResourceType } from "@fastcart/shared-types";

import { EmptyState } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { listStoreLocales } from "@/lib/locales";
import { PERMISSIONS } from "@/lib/permissions";
import {
  collectTranslationFields,
  deleteTranslation,
  isTranslationNotFound,
  listTranslations,
  mergeTranslationFields,
  translationCompleteness,
  translationKey,
  translationText,
  upsertTranslation,
  type TranslationField,
} from "@/lib/translations";
import { useAsyncData } from "@/lib/use-async-data";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-60";

/** Demo locales so the panel is clickable without a server. */
function demoLocales(storeId: string): StoreLocale[] {
  const now = Math.floor(Date.now() / 1000);
  return [
    {
      store_id: storeId,
      locale: "en-US",
      is_default: true,
      label: "English (United States)",
      direction: "ltr",
      created_at: now,
      updated_at: now,
    },
    {
      store_id: storeId,
      locale: "fr-FR",
      is_default: false,
      label: "Français",
      direction: "ltr",
      created_at: now,
      updated_at: now,
    },
    {
      store_id: storeId,
      locale: "de-DE",
      is_default: false,
      label: "Deutsch",
      direction: "ltr",
      created_at: now,
      updated_at: now,
    },
  ];
}

export interface TranslationEditorProps {
  /** Resource the translation belongs to. */
  resourceType: TranslationResourceType;
  resourceId: string;
  /** Fields offered for translation, in display order. */
  fields: readonly TranslationField[];
  /**
   * Source content of the resource. Shown as the fallback for empty fields and
   * in the storefront preview.
   */
  source?: Record<string, unknown>;
}

/**
 * Edits one locale's translation of a content resource.
 *
 * The store's default locale is excluded: it carries the source content, and
 * other locales override it field by field. Fields left blank are omitted from
 * the stored translation so the storefront falls back to the source value.
 */
export function TranslationEditor({
  resourceType,
  resourceId,
  fields,
  source = {},
}: TranslationEditorProps) {
  const { mode, store, dataKey, can } = useAuth();
  const writable = can(PERMISSIONS.settings.write);
  const fieldIdPrefix = useId().replace(/:/g, "");

  // Derive the keys from a string signature so callers may pass a fresh array
  // on every render without resetting the draft while the user types.
  const keySignature = fields.map((field) => field.key).join("\u0000");
  const keys = useMemo(
    () => (keySignature.length > 0 ? keySignature.split("\u0000") : []),
    [keySignature],
  );

  const [selectedLocale, setSelectedLocale] = useState("");
  const [row, setRow] = useState<Translation | null>(null);
  const [rowLoading, setRowLoading] = useState(false);
  const [rowError, setRowError] = useState<string | null>(null);
  const [rowVersion, setRowVersion] = useState(0);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<"save" | "delete" | null>(null);
  // Demo translations live in component state so the flow works without an API.
  const [demoRows, setDemoRows] = useState<Record<string, Translation>>({});

  useEffect(() => {
    setSelectedLocale("");
    setRow(null);
    setRowError(null);
    setDemoRows({});
  }, [store?.id, dataKey]);

  const localeLoader = useMemo(
    () =>
      (signal: AbortSignal): Promise<StoreLocale[]> => {
        if (mode === "demo") return Promise.resolve(demoLocales(store?.id ?? ""));
        if (!store) return Promise.reject(new Error("No store is selected."));
        return listStoreLocales(store.id, signal);
      },
    [mode, store],
  );

  const {
    data: locales,
    error: localesError,
    loading: localesLoading,
    reload: reloadLocales,
  } = useAsyncData(localeLoader, [mode, store?.id, dataKey]);

  /** The default locale holds the source content, so it is not translated. */
  const selectableLocales = useMemo(
    () => (locales ?? []).filter((locale) => !locale.is_default),
    [locales],
  );

  useEffect(() => {
    if (selectableLocales.length === 0) {
      setSelectedLocale("");
      return;
    }
    if (!selectableLocales.some((locale) => locale.locale === selectedLocale)) {
      setSelectedLocale(selectableLocales[0]?.locale ?? "");
    }
  }, [selectableLocales, selectedLocale]);

  const demoRow =
    selectedLocale.length > 0
      ? (demoRows[translationKey(resourceType, resourceId, selectedLocale)] ?? null)
      : null;

  useEffect(() => {
    if (selectedLocale.length === 0) {
      setRow(null);
      setRowLoading(false);
      setRowError(null);
      return;
    }
    if (mode === "demo") {
      setRow(demoRow);
      setRowLoading(false);
      setRowError(null);
      return;
    }
    if (!store) {
      setRow(null);
      setRowLoading(false);
      setRowError("No store is selected.");
      return;
    }

    const controller = new AbortController();
    setRow(null);
    setRowLoading(true);
    setRowError(null);
    listTranslations(
      store.id,
      { resource_type: resourceType, resource_id: resourceId, locale: selectedLocale },
      controller.signal,
    )
      .then((rows) => {
        if (controller.signal.aborted) return;
        setRow(rows[0] ?? null);
        setRowLoading(false);
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        // A missing row is the normal "not translated yet" state, not a failure.
        if (isTranslationNotFound(cause)) {
          setRow(null);
        } else {
          setRowError(errorMessage(cause));
        }
        setRowLoading(false);
      });
    return () => controller.abort();
  }, [mode, store, resourceType, resourceId, selectedLocale, demoRow, rowVersion]);

  /** Resets the draft whenever the locale, the loaded row or the resource changes. */
  useEffect(() => {
    const values: Record<string, string> = {};
    for (const key of keys) values[key] = translationText(row?.fields[key]);
    setDraft(values);
  }, [row, selectedLocale, keySignature, keys]);

  const draftFields = collectTranslationFields(draft, keys);
  const completeness = translationCompleteness({ fields: draftFields }, keys);
  const preview = mergeTranslationFields(source, { fields: draftFields }, keys);

  const baseline = useMemo(() => {
    const values: Record<string, string> = {};
    for (const key of keys) values[key] = translationText(row?.fields[key]);
    return JSON.stringify(values);
  }, [row, keySignature, keys]);

  const dirty = JSON.stringify(draft) !== baseline;
  const hasContent = keys.some((key) => (draft[key] ?? "").trim().length > 0);
  // Saving an empty draft would store a translation that translates nothing, so
  // clearing every field of an existing row deletes it instead.
  const clearsRow = !hasContent && row !== null;
  const canSave = writable && !busy && !rowLoading && dirty && (hasContent || clearsRow);
  const canDelete = writable && !busy && !rowLoading && row !== null;

  const clearDemoRow = (locale: string) => {
    const key = translationKey(resourceType, resourceId, locale);
    setDemoRows((current) => {
      const next = { ...current };
      delete next[key];
      return next;
    });
  };

  const save = async () => {
    if (!selectedLocale || !canSave) return;
    if (clearsRow) {
      await remove();
      return;
    }
    setBusy("save");
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        const key = translationKey(resourceType, resourceId, selectedLocale);
        setDemoRows((current) => {
          const existing = current[key];
          return {
            ...current,
            [key]: {
              id: existing?.id ?? `tr_demo_${Date.now()}`,
              resource_type: resourceType,
              resource_id: resourceId,
              locale: selectedLocale,
              fields: draftFields,
              created_at: existing?.created_at ?? now,
              updated_at: now,
            },
          };
        });
        toast.success(`${selectedLocale} translation saved in the demo`);
        return;
      }
      if (!store) throw new Error("No store is selected.");
      const saved = await upsertTranslation(
        store.id,
        { resource_type: resourceType, resource_id: resourceId, locale: selectedLocale },
        { fields: draftFields },
      );
      setRow(saved);
      toast.success(`${selectedLocale} translation saved`);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!selectedLocale || !canDelete) return;
    setBusy("delete");
    try {
      if (mode === "demo") {
        clearDemoRow(selectedLocale);
        toast.success(`${selectedLocale} translation removed in the demo`);
        return;
      }
      if (!store) throw new Error("No store is selected.");
      try {
        await deleteTranslation(store.id, {
          resource_type: resourceType,
          resource_id: resourceId,
          locale: selectedLocale,
        });
      } catch (cause) {
        // A concurrent delete already removed the row; the outcome is the same.
        if (!isTranslationNotFound(cause)) throw cause;
      }
      setRow(null);
      toast.success(`${selectedLocale} translation removed`);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  if (localesLoading && !locales) {
    return (
      <p className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
        <Loader2 className="size-3 animate-spin" aria-hidden="true" />
        Loading locales…
      </p>
    );
  }

  if (localesError && !locales) {
    return (
      <div
        role="alert"
        className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-[11.5px] text-destructive"
      >
        <span>{localesError}</span>
        <Button variant="outline" size="sm" onClick={reloadLocales}>
          Try again
        </Button>
      </div>
    );
  }

  if (selectableLocales.length === 0) {
    return (
      <EmptyState
        title="No translation locale"
        description="Add a second locale under Settings → Locales. The default locale holds the source content."
      />
    );
  }

  const activeLocale = selectableLocales.find((locale) => locale.locale === selectedLocale);
  const completenessVariant =
    completeness.translated === completeness.total
      ? "brand"
      : completeness.translated > 0
        ? "warning"
        : "secondary";

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="w-full space-y-2 sm:w-[240px]">
          <Label htmlFor={`${fieldIdPrefix}-locale`}>Locale</Label>
          <div className="relative">
            <select
              id={`${fieldIdPrefix}-locale`}
              value={selectedLocale}
              onChange={(event) => setSelectedLocale(event.target.value)}
              className={selectClass}
            >
              {selectableLocales.map((locale) => (
                <option key={locale.locale} value={locale.locale}>
                  {locale.label ? `${locale.label} (${locale.locale})` : locale.locale}
                </option>
              ))}
            </select>
            <ChevronDown
              className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
              aria-hidden="true"
            />
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Badge variant={completenessVariant}>
            {completeness.translated}/{completeness.total} fields translated
          </Badge>
          {!writable ? <Badge variant="outline">Read-only</Badge> : null}
        </div>
      </div>

      {rowLoading ? (
        <p className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
          <Loader2 className="size-3 animate-spin" aria-hidden="true" />
          Loading translation…
        </p>
      ) : rowError ? (
        <div
          role="alert"
          className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-[11.5px] text-destructive"
        >
          <span>{rowError}</span>
          <Button variant="outline" size="sm" onClick={() => setRowVersion((value) => value + 1)}>
            <RotateCcw className="size-3.5" aria-hidden="true" />
            Try again
          </Button>
        </div>
      ) : row === null ? (
        <div className="flex items-start gap-2 rounded-lg border border-dashed border-border px-3 py-3">
          <Languages className="mt-0.5 size-3.5 shrink-0 text-faint" aria-hidden="true" />
          <div>
            <p className="text-[12px] font-medium text-foreground">
              Untranslated · {activeLocale?.label ?? selectedLocale}
            </p>
            <p className="mt-0.5 text-[11.5px] text-muted-foreground">
              No translation exists for this locale yet. Fields left empty fall back to the source
              content on the storefront.
            </p>
          </div>
        </div>
      ) : (
        <p className="text-[11.5px] text-muted-foreground">
          Editing the {activeLocale?.label ?? selectedLocale} translation. Clear a field to fall
          back to the source content.
        </p>
      )}

      <form
        className="space-y-4"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        {fields.map((field) => {
          const inputId = `${fieldIdPrefix}-${field.key}`;
          const fallback = translationText(source[field.key]);
          const value = draft[field.key] ?? "";
          return (
            <div key={field.key} className="space-y-2">
              <Label htmlFor={inputId}>{field.label}</Label>
              {field.multiline ? (
                <textarea
                  id={inputId}
                  rows={3}
                  value={value}
                  onChange={(event) =>
                    setDraft((current) => ({ ...current, [field.key]: event.target.value }))
                  }
                  placeholder={fallback}
                  disabled={busy !== null || rowLoading || !writable}
                  className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs transition-[color,box-shadow,border-color] outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
                />
              ) : (
                <Input
                  id={inputId}
                  value={value}
                  onChange={(event) =>
                    setDraft((current) => ({ ...current, [field.key]: event.target.value }))
                  }
                  placeholder={fallback}
                  autoComplete="off"
                  disabled={busy !== null || rowLoading || !writable}
                />
              )}
              {fallback.length > 0 ? (
                <p className="truncate text-[11px] text-faint">Falls back to “{fallback}”</p>
              ) : null}
            </div>
          );
        })}

        {Object.keys(source).length > 0 ? (
          <div className="rounded-lg border border-border bg-secondary/30 px-3 py-3">
            <p className="text-[11.5px] font-medium text-ink-soft">Storefront preview</p>
            <dl className="mt-2 space-y-1">
              {fields.map((field) => (
                <div key={field.key} className="flex gap-2 text-[11.5px]">
                  <dt className="w-[110px] shrink-0 text-faint">{field.label}</dt>
                  <dd className="min-w-0 flex-1 truncate text-foreground">
                    {translationText(preview[field.key]) || "—"}
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        ) : null}

        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-4">
          <Button
            type="button"
            variant="destructive"
            size="sm"
            onClick={() => void remove()}
            disabled={!canDelete}
          >
            {busy === "delete" ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <Trash2 className="size-3.5" aria-hidden="true" />
            )}
            Delete translation
          </Button>
          <Button type="submit" size="sm" disabled={!canSave}>
            {busy === "save" ? (
              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
            ) : (
              <Save className="size-3.5" aria-hidden="true" />
            )}
            Save translation
          </Button>
        </div>
      </form>
    </div>
  );
}
