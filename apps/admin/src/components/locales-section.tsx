import { useCallback, useEffect, useState } from "react";
import { Globe2, Loader2, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { StoreLocale } from "@fastcart/shared-types";

import { EmptyState } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import {
  addStoreLocale,
  listStoreLocales,
  localeInputError,
  removeStoreLocale,
  setDefaultStoreLocale,
} from "@/lib/locales";
import { PERMISSIONS } from "@/lib/permissions";
import { useAsyncData } from "@/lib/use-async-data";

/** Demo locales so the section renders without a server. */
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
  ];
}

/**
 * Display locales for the store.
 *
 * Translations are keyed by these locale tags; removing one leaves existing
 * translation rows in place so the locale can be re-added without loss.
 */
export function LocalesSection() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoData, setDemoData] = useState<StoreLocale[]>([]);
  const [draft, setDraft] = useState("");
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [busyLocale, setBusyLocale] = useState<string | null>(null);

  const writable = can(PERMISSIONS.settings.write);

  useEffect(() => {
    setDraft("");
    setFieldError(null);
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<StoreLocale[]> => {
      if (mode === "demo") {
        return Promise.resolve(demoData.length > 0 ? demoData : demoLocales(store?.id ?? ""));
      }
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listStoreLocales(store.id, signal);
    },
    [mode, store, demoData],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoData,
  ]);

  const add = async () => {
    if (busy) return;
    const message = localeInputError(draft);
    if (message !== null) {
      setFieldError(message);
      return;
    }
    setBusy(true);
    try {
      const locale = draft.trim().replace(/_/g, "-");
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        setDemoData((current) => {
          const base = current.length > 0 ? current : demoLocales(store?.id ?? "");
          return [
            ...base,
            {
              store_id: store?.id ?? "",
              locale,
              is_default: false,
              label: null,
              direction: "ltr" as const,
              created_at: now,
              updated_at: now,
            },
          ];
        });
        setDraft("");
        toast.success("Locale added in the demo");
        return;
      }
      if (!store) throw new Error("No store is selected.");
      await addStoreLocale(store.id, { locale });
      setDraft("");
      toast.success(`${locale} added`);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const promote = async (locale: StoreLocale) => {
    if (busyLocale || locale.is_default) return;
    setBusyLocale(locale.locale);
    try {
      if (mode === "demo") {
        setDemoData((current) =>
          (current.length > 0 ? current : demoLocales(store?.id ?? "")).map((entry) => ({
            ...entry,
            is_default: entry.locale === locale.locale,
          })),
        );
        toast.success("Default locale updated in the demo");
        return;
      }
      if (!store) throw new Error("No store is selected.");
      await setDefaultStoreLocale(store.id, locale.locale);
      toast.success(`${locale.locale} is now the default locale`);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyLocale(null);
    }
  };

  const remove = async (locale: StoreLocale) => {
    if (busyLocale || locale.is_default) return;
    setBusyLocale(locale.locale);
    try {
      if (mode === "demo") {
        setDemoData((current) =>
          (current.length > 0 ? current : demoLocales(store?.id ?? "")).filter(
            (entry) => entry.locale !== locale.locale,
          ),
        );
        toast.success("Locale removed in the demo");
        return;
      }
      if (!store) throw new Error("No store is selected.");
      await removeStoreLocale(store.id, locale.locale);
      toast.success(`${locale.locale} removed`);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyLocale(null);
    }
  };

  return (
    <>
      <div className="border-b border-border px-6 py-5">
        <h2 className="text-[14px] font-medium text-foreground">Locales</h2>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          Languages this store publishes in. The default locale is the fallback for untranslated
          content.
        </p>
      </div>

      <div className="space-y-4 p-6">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-end">
          <div className="w-full space-y-2 sm:w-[280px]">
            <Label htmlFor="store-locale">Add a locale</Label>
            <Input
              id="store-locale"
              value={draft}
              onChange={(event) => {
                setDraft(event.target.value);
                setFieldError(null);
              }}
              placeholder="fr-FR"
              autoComplete="off"
              spellCheck={false}
              disabled={busy || !writable}
              className="font-mono"
              aria-invalid={fieldError !== null}
            />
            {fieldError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {fieldError}
              </p>
            ) : null}
          </div>
          <Button
            variant="outline"
            className="shrink-0"
            onClick={() => void add()}
            disabled={busy || !writable}
          >
            {busy ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Plus className="size-4" aria-hidden="true" />
            )}
            Add locale
          </Button>
        </div>

        {loading && !data ? (
          <p className="text-[11.5px] text-muted-foreground">Loading locales…</p>
        ) : error && !data ? (
          <p className="text-[11.5px] text-destructive" role="alert">
            {error}
          </p>
        ) : !data || data.length === 0 ? (
          <EmptyState
            title="No locales configured"
            description="Add at least one locale; the first one becomes the default."
          />
        ) : (
          <div className="overflow-hidden rounded-lg border border-border">
            <Table>
              <TableHeader>
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-4">Locale</TableHead>
                  <TableHead className="hidden w-[150px] sm:table-cell">Label</TableHead>
                  <TableHead className="w-[110px]">Direction</TableHead>
                  <TableHead className="hidden w-[120px] md:table-cell">Default</TableHead>
                  <TableHead className="w-[180px] pr-4 text-right" aria-label="Actions" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.map((locale) => (
                  <TableRow key={locale.locale}>
                    <TableCell className="pl-4">
                      <p className="flex items-center gap-2 font-mono text-[12px] text-foreground">
                        <Globe2 className="size-3.5 text-faint" aria-hidden="true" />
                        {locale.locale}
                      </p>
                    </TableCell>
                    <TableCell className="hidden text-[11.5px] text-muted-foreground sm:table-cell">
                      {locale.label ?? "—"}
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">{locale.direction.toUpperCase()}</Badge>
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      {locale.is_default ? <Badge variant="brand">Default</Badge> : null}
                    </TableCell>
                    <TableCell className="pr-4 text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          disabled={!writable || locale.is_default || busyLocale !== null}
                          onClick={() => void promote(locale)}
                        >
                          {busyLocale === locale.locale ? (
                            <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                          ) : null}
                          Make default
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          aria-label={`Remove ${locale.locale}`}
                          disabled={!writable || locale.is_default || busyLocale !== null}
                          onClick={() => void remove(locale)}
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
    </>
  );
}
