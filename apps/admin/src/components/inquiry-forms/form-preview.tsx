import { Paperclip } from "lucide-react";
import type { InquiryFormField, InquiryFormSettings } from "@fastcart/shared-types";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

function PreviewField({ field }: { field: InquiryFormField }) {
  const id = `preview-${field.key}`;
  const inputType =
    field.kind === "email"
      ? "email"
      : field.kind === "phone"
        ? "tel"
        : field.kind === "number"
          ? "number"
          : field.kind === "date"
            ? "date"
            : "text";

  if (field.kind === "consent") {
    return (
      <label className="flex items-start gap-2.5 rounded-md border border-input bg-card px-3 py-2.5 text-[11.5px] leading-relaxed text-ink-soft">
        <input
          type="checkbox"
          disabled
          defaultChecked={field.required}
          className="mt-0.5 size-3.5"
        />
        <span>{field.help ?? field.label}</span>
      </label>
    );
  }

  return (
    <div className="space-y-1.5">
      <Label htmlFor={id} className="text-[11.5px] text-foreground">
        {field.label || "Untitled field"}
        {field.required ? <span className="text-destructive">*</span> : null}
      </Label>

      {field.kind === "textarea" ? (
        <textarea
          id={id}
          disabled
          rows={3}
          placeholder={field.placeholder ?? ""}
          className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[12.5px] text-foreground opacity-100 disabled:cursor-default disabled:opacity-100"
        />
      ) : field.kind === "select" ? (
        <select
          id={id}
          disabled
          className="h-9 w-full appearance-none rounded-md border border-input bg-card px-3 text-[12.5px] text-faint disabled:cursor-default disabled:opacity-100"
        >
          <option value="">{field.placeholder ?? "Choose an option"}</option>
          {field.options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      ) : field.kind === "radio" ? (
        <div className="space-y-1.5 rounded-md border border-input bg-card px-3 py-2.5">
          {field.options.map((option) => (
            <label key={option.value} className="flex items-center gap-2 text-[12px] text-ink-soft">
              <input type="radio" disabled className="size-3.5" />
              {option.label}
            </label>
          ))}
          {field.options.length === 0 ? (
            <p className="text-[11.5px] text-faint">No options yet</p>
          ) : null}
        </div>
      ) : field.kind === "checkbox" ? (
        <div className="space-y-1.5 rounded-md border border-input bg-card px-3 py-2.5">
          {field.options.map((option) => (
            <label key={option.value} className="flex items-center gap-2 text-[12px] text-ink-soft">
              <input type="checkbox" disabled className="size-3.5" />
              {option.label}
            </label>
          ))}
          {field.options.length === 0 ? (
            <p className="text-[11.5px] text-faint">No options yet</p>
          ) : null}
        </div>
      ) : field.kind === "file" ? (
        <div className="flex items-center gap-2 rounded-md border border-dashed border-line-strong bg-background px-3 py-4 text-[11.5px] text-faint">
          <Paperclip className="size-3.5" aria-hidden="true" />
          {field.file
            ? `Up to ${field.file.max_files} files, ${Math.round(
                (field.file.max_bytes / (1024 * 1024)) * 10,
              ) / 10} MB each`
            : "File upload"}
        </div>
      ) : (
        <Input
          id={id}
          type={inputType}
          disabled
          placeholder={field.placeholder ?? ""}
          className="h-9 text-[12.5px] disabled:cursor-default disabled:opacity-100"
        />
      )}

      {field.help ? <p className="text-[10.5px] text-faint">{field.help}</p> : null}
    </div>
  );
}

export function FormPreview({
  fields,
  settings,
}: {
  fields: InquiryFormField[];
  settings: InquiryFormSettings;
}) {
  const visible = fields.filter(
    (field) => field.kind !== "hidden" && (field.kind !== "file" || settings.allow_attachments),
  );
  const hasConsentField = fields.some((field) => field.kind === "consent");
  const showSettingsConsent =
    !hasConsentField && settings.consent_text !== null && settings.consent_text !== undefined;

  return (
    <div className="rounded-xl border border-border bg-card p-5 shadow-soft">
      <div className="flex items-center justify-between gap-2 border-b border-border pb-3">
        <div>
          <p className="text-[12.5px] font-medium text-foreground">Live preview</p>
          <p className="mt-0.5 text-[10.5px] text-faint">
            {visible.length} visible {visible.length === 1 ? "field" : "fields"}
          </p>
        </div>
        <span className="rounded-full border border-border bg-secondary px-2 py-0.5 text-[10px] text-muted-foreground">
          Storefront
        </span>
      </div>

      <div className="mt-4 space-y-3.5">
        {visible.map((field) => (
          <PreviewField key={field.key} field={field} />
        ))}

        {showSettingsConsent ? (
          <label className="flex items-start gap-2 text-[11.5px] leading-relaxed text-ink-soft">
            <input
              type="checkbox"
              disabled
              defaultChecked={Boolean(settings.consent_required)}
              className="mt-0.5 size-3.5"
            />
            <span>{settings.consent_text}</span>
          </label>
        ) : null}

        {visible.length === 0 ? (
          <p className="rounded-lg border border-dashed border-border px-3 py-8 text-center text-[11.5px] text-faint">
            Fields you add will appear here.
          </p>
        ) : null}

        <button
          type="button"
          disabled
          className={cn(
            "mt-1 h-9 w-full rounded-md bg-primary text-[12.5px] font-medium text-primary-foreground",
            "disabled:cursor-default disabled:opacity-100",
          )}
        >
          {settings.submit_label || "Submit"}
        </button>
      </div>
    </div>
  );
}
