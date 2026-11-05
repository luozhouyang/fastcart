import { useEffect, useMemo, useState } from "react";
import { AlertCircle, Loader2, Search } from "lucide-react";
import type { AdminClient } from "@fastcart/admin-sdk";
import type {
  DataPickerItem,
  DataSourceQuery,
  JsonValue,
  ThemeEditorField,
  ThemeEditorManifest,
} from "@fastcart/shared-types";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/api";
import {
  bindingId,
  componentFields,
  isJsonObject,
  parseJsonText,
  prettyJson,
} from "@/lib/theme-editor";

const textareaClass =
  "min-h-[96px] w-full resize-y rounded-md border border-input bg-card px-3 py-2 font-mono text-[11.5px] leading-relaxed text-foreground shadow-xs outline-none transition-[color,box-shadow,border-color] placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25";

const selectClass =
  "h-9 w-full cursor-pointer appearance-none rounded-md border border-input bg-card px-3 text-[12.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25";

function humanize(key: string): string {
  const spaced = key.replace(/[_-]+/g, " ").trim();
  return spaced.length === 0 ? key : spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/** Resolves the data source from `source`, falling back to picker metadata. */
function fieldSource(field: ThemeEditorField): string | null {
  if (typeof field.source === "string" && field.source.length > 0) return field.source;
  if (isJsonObject(field.picker)) {
    for (const key of ["source", "kind", "data_source"]) {
      const value = field.picker[key];
      if (typeof value === "string" && value.length > 0) return value;
    }
  }
  return field.type === "asset" ? "asset" : null;
}

/** Extracts supported query defaults from an editor picker config. */
function pickerQuery(picker: JsonValue | null | undefined): DataSourceQuery {
  if (!isJsonObject(picker)) return {};
  const search = picker.search;
  const limit = picker.limit;
  const offset = picker.offset;
  const collectionId = picker.collection_id;
  const tag = picker.tag;
  const published = picker.published;
  const sort = picker.sort;
  return {
    ...(typeof search === "string" ? { search } : {}),
    ...(typeof limit === "number" ? { limit } : {}),
    ...(typeof offset === "number" ? { offset } : {}),
    ...(typeof collectionId === "string" ? { collection_id: collectionId } : {}),
    ...(typeof tag === "string" ? { tag } : {}),
    ...(typeof published === "boolean" ? { published } : {}),
    ...(typeof sort === "string" ? { sort } : {}),
  };
}

function textValue(value: JsonValue | undefined): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return prettyJson(value);
}

function numberValue(value: JsonValue | undefined): string {
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  if (typeof value === "string" && value.trim().length > 0) return value;
  return "";
}

function JsonField({
  value,
  onChange,
}: {
  value: JsonValue | undefined;
  onChange: (value: JsonValue | undefined) => void;
}) {
  const serialized = useMemo(() => prettyJson(value ?? null), [value]);
  const [text, setText] = useState(serialized);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setText(serialized);
    setError(null);
  }, [serialized]);

  const update = (next: string) => {
    setText(next);
    const parsed = parseJsonText(next);
    if (parsed.ok) {
      setError(null);
      onChange(parsed.value === null ? undefined : parsed.value);
    } else {
      setError(parsed.error ?? "Invalid JSON");
    }
  };

  return (
    <div className="space-y-1">
      <textarea
        value={text}
        onChange={(event) => update(event.target.value)}
        spellCheck={false}
        aria-invalid={error !== null}
        className={textareaClass}
      />
      {error ? <p className="text-[10.5px] text-destructive">Invalid JSON: {error}</p> : null}
    </div>
  );
}

function DataPickerField({
  client,
  source,
  picker,
  value,
  onChange,
}: {
  client: AdminClient | null;
  source: string;
  picker?: JsonValue | null | undefined;
  value: JsonValue | undefined;
  onChange: (value: JsonValue | undefined) => void;
}) {
  const currentId = bindingId(value);
  const [search, setSearch] = useState("");
  const [items, setItems] = useState<DataPickerItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!client) {
      setItems([]);
      return;
    }
    let active = true;
    const controller = new AbortController();
    const defaults = pickerQuery(picker);
    const timer = window.setTimeout(() => {
      const trimmed = search.trim();
      const query: DataSourceQuery = {
        ...defaults,
        limit: defaults.limit ?? 50,
        ...(trimmed.length > 0 ? { search: trimmed } : {}),
      };
      setLoading(true);
      setError(null);
      client.dataSources
        .list(source, query, { signal: controller.signal })
        .then((response) => {
          if (active) setItems(response.items);
        })
        .catch((cause: unknown) => {
          if (active && !controller.signal.aborted) setError(errorMessage(cause));
        })
        .finally(() => {
          if (active) setLoading(false);
        });
    }, 250);

    return () => {
      active = false;
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [client, source, picker, search]);

  if (!client) {
    return (
      <div className="space-y-1">
        <Input
          value={currentId ?? ""}
          onChange={(event) => {
            const id = event.target.value.trim();
            onChange(id.length === 0 ? undefined : { source, id });
          }}
          placeholder={`${source} identifier`}
          className="h-9 font-mono text-[11.5px]"
        />
        <p className="text-[10px] text-faint">
          Demo mode: enter a reference id, or connect a store to search.
        </p>
      </div>
    );
  }

  const known = currentId !== null && items.some((item) => item.id === currentId);

  return (
    <div className="space-y-1.5">
      <div className="relative">
        <Search
          className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-faint"
          aria-hidden="true"
        />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder={`Search ${source.replace(/_/g, " ")}…`}
          className="h-9 pl-8 text-[12px]"
        />
      </div>
      <select
        value={currentId ?? ""}
        onChange={(event) => {
          const id = event.target.value;
          const item = items.find((entry) => entry.id === id);
          onChange(item ? { source, id: item.id } : undefined);
        }}
        className={selectClass}
        aria-label={`${source} reference`}
      >
        <option value="">Not selected</option>
        {currentId !== null && !known ? <option value={currentId}>{currentId}</option> : null}
        {items.map((item) => (
          <option key={item.id} value={item.id}>
            {item.label}
            {item.handle ? ` · /${item.handle}` : ""}
          </option>
        ))}
      </select>
      <div className="flex items-center gap-1.5 text-[10px] text-faint">
        {loading ? <Loader2 className="size-3 animate-spin" aria-hidden="true" /> : null}
        {error ? (
          <>
            <AlertCircle className="size-3 text-destructive" aria-hidden="true" />
            <span className="truncate text-destructive">{error}</span>
          </>
        ) : (
          <span>
            {items.length} {items.length === 1 ? "record" : "records"} available
            {currentId ? ` · selected ${currentId}` : ""}
          </span>
        )}
      </div>
    </div>
  );
}

function InquiryFormField({
  value,
  onChange,
}: {
  value: JsonValue | undefined;
  onChange: (value: JsonValue | undefined) => void;
}) {
  const current = isJsonObject(value) && typeof value.handle === "string" ? value.handle : "";
  return (
    <div className="space-y-1">
      <Input
        value={current}
        onChange={(event) => {
          const handle = event.target.value.trim();
          onChange(handle.length === 0 ? undefined : { source: "inquiry_form", handle });
        }}
        placeholder="contact"
        className="h-9 font-mono text-[12px]"
      />
      <p className="text-[10px] text-faint">
        Handle of a published inquiry form, for example `contact`.
      </p>
    </div>
  );
}

function FieldControl({
  fieldKey,
  field,
  value,
  onChange,
  client,
}: {
  fieldKey: string;
  field: ThemeEditorField;
  value: JsonValue | undefined;
  onChange: (value: JsonValue | undefined) => void;
  client: AdminClient | null;
}) {
  switch (field.type) {
    case "text":
      return (
        <Input
          value={textValue(value)}
          onChange={(event) => onChange(event.target.value)}
          className="h-9"
        />
      );
    case "textarea":
      return (
        <textarea
          value={textValue(value)}
          onChange={(event) => onChange(event.target.value)}
          spellCheck={false}
          className={textareaClass}
        />
      );
    case "number":
      return (
        <Input
          type="number"
          value={numberValue(value)}
          min={field.min ?? undefined}
          max={field.max ?? undefined}
          step="any"
          onChange={(event) => {
            const text = event.target.value;
            if (text.trim().length === 0) {
              onChange(undefined);
              return;
            }
            const parsed = Number(text);
            onChange(Number.isFinite(parsed) ? parsed : undefined);
          }}
          className="h-9"
        />
      );
    case "boolean":
      return (
        <div className="flex h-9 items-center">
          <Switch
            checked={value === true}
            onCheckedChange={(checked) => onChange(checked)}
            aria-label={humanize(fieldKey)}
          />
        </div>
      );
    case "select":
      return (
        <select
          value={typeof value === "string" ? value : ""}
          onChange={(event) =>
            onChange(event.target.value.length > 0 ? event.target.value : undefined)
          }
          className={selectClass}
        >
          <option value="">Not set</option>
          {(field.options ?? []).map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      );
    case "data_source": {
      const source = fieldSource(field) ?? "";
      if (source === "inquiry_form") {
        return <InquiryFormField value={value} onChange={onChange} />;
      }
      if (source.length === 0 || source === "product_list" || source === "search") {
        return (
          <div className="space-y-1">
            <JsonField value={value} onChange={onChange} />
            <p className="text-[10px] text-faint">
              {source === "product_list"
                ? 'Provide a product-list binding object, for example { "collection_id": "col_1", "limit": 8 }.'
                : source === "search"
                  ? 'Provide a search binding object, for example { "query": "linen" }.'
                  : "Provide the binding object consumed by the theme."}
            </p>
          </div>
        );
      }
      return (
        <DataPickerField
          client={client}
          source={source}
          picker={field.picker}
          value={value}
          onChange={onChange}
        />
      );
    }
    case "asset": {
      const source = fieldSource(field) ?? "asset";
      return (
        <DataPickerField
          client={client}
          source={source}
          picker={field.picker}
          value={value}
          onChange={onChange}
        />
      );
    }
    case "object":
    case "array":
    case "custom":
    default:
      return <JsonField value={value} onChange={onChange} />;
  }
}

export function BlockFields({
  manifest,
  type,
  props,
  onChange,
  client,
}: {
  manifest: ThemeEditorManifest | null;
  type: string;
  props: Record<string, JsonValue>;
  onChange: (key: string, value: JsonValue | undefined) => void;
  client: AdminClient | null;
}) {
  const fields = componentFields(manifest, type);
  const entries = Object.entries(fields);

  if (entries.length === 0) {
    return (
      <div className="rounded-xl border border-dashed border-border px-4 py-8 text-center">
        <p className="text-[12.5px] font-medium text-foreground">No configurable props</p>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          This component does not declare editable fields in the theme manifest.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {entries.map(([key, field]) => (
        <div key={key} className="space-y-1.5">
          <div className="flex items-center justify-between gap-2">
            <Label htmlFor={`field-${key}`} className="text-[12px]">
              {field.label ?? humanize(key)}
              {field.required ? <span className="ml-0.5 text-destructive">*</span> : null}
            </Label>
            <span className="font-mono text-[9.5px] text-faint">{field.type}</span>
          </div>
          {field.description ? (
            <p className="text-[10.5px] text-faint">{field.description}</p>
          ) : null}
          <div id={`field-${key}`}>
            <FieldControl
              fieldKey={key}
              field={field}
              value={props[key]}
              onChange={(value) => onChange(key, value)}
              client={client}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
