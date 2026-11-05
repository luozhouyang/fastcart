import type {
  CreateInquiryRequest,
  InquiryFormField,
  StoreInquiryForm,
} from "@fastcart/shared-types";
import { createStorefrontClient } from "@fastcart/storefront-sdk";
import { useMemo, useState, type FormEvent } from "react";

import { useThemeRuntime } from "../context";

/** Props accepted by the default theme contact form. */
export interface ContactFormProps {
  /** Published inquiry form resolved from the `inquiry_form` data binding. */
  form?: StoreInquiryForm;
  /** API origin when the storefront is served from a different host. */
  apiBaseUrl?: string;
  /** Optional heading override. */
  title?: string;
  /** Optional description shown above the fields. */
  description?: string;
  /** Additional class names for the wrapping form. */
  className?: string;
  /** Called after a submission is accepted. */
  onSubmitted?: (id: string) => void;
}

type FieldValue = string | string[];
type FormStatus = "idle" | "submitting" | "success" | "error";

const CORE_FIELDS = new Set(["name", "email", "phone", "subject", "message"]);

/**
 * Renders a published inquiry form and submits it through the storefront SDK.
 *
 * The component is intentionally framework-light: it depends only on React and
 * the public storefront contracts, so a theme can wrap or restyle it without
 * reaching into the inquiry module.
 */
export function ContactForm(props: ContactFormProps) {
  if (!props.form) {
    return <p>Contact form is not configured for this page.</p>;
  }
  return <ContactFormBody {...props} form={props.form} />;
}

function ContactFormBody({
  form,
  apiBaseUrl,
  title,
  description,
  className,
  onSubmitted,
}: ContactFormProps & { form: StoreInquiryForm }) {
  const consentField = form.fields.find((field) => field.kind === "consent");
  const consentRequired = form.settings.consent_required || Boolean(consentField?.required);
  const consentText =
    form.settings.consent_text ?? consentField?.label ?? "I agree to the privacy terms.";
  // The theme runtime supplies the API origin when the storefront is served
  // from a different host (or behind a reverse proxy). An explicit prop still
  // wins so a page can override the context.
  const runtime = useThemeRuntime();
  const resolvedApiBaseUrl = apiBaseUrl ?? runtime.apiBaseUrl;
  const [values, setValues] = useState<Record<string, FieldValue>>(() =>
    initialValues(form.fields),
  );
  const [files, setFiles] = useState<Record<string, File[]>>({});
  const [consent, setConsent] = useState(false);
  const [status, setStatus] = useState<FormStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  const client = useMemo(
    () => createStorefrontClient(resolvedApiBaseUrl ? { baseUrl: resolvedApiBaseUrl } : {}),
    [resolvedApiBaseUrl],
  );

  function setValue(key: string, value: FieldValue) {
    setValues((current) => ({ ...current, [key]: value }));
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (status === "submitting") {
      return;
    }
    setStatus("submitting");
    setError(null);

    try {
      const uploaded: Record<string, string[]> = {};
      for (const field of form.fields) {
        if (field.kind !== "file") {
          continue;
        }
        const selected = files[field.key] ?? [];
        if (selected.length === 0) {
          continue;
        }
        const ids: string[] = [];
        for (const file of selected) {
          const result = await client.inquiryFiles.upload(file, form.handle, field.key);
          ids.push(result.id);
        }
        uploaded[field.key] = ids;
      }

      const body: CreateInquiryRequest = {
        form_handle: form.handle,
        files: uploaded,
        data: {},
        consent,
      };
      if (typeof window !== "undefined") {
        body.page_url = window.location.href;
      }
      if (typeof navigator !== "undefined") {
        body.locale = navigator.language;
      }

      const custom: Record<string, unknown> = {};
      for (const field of form.fields) {
        if (field.kind === "file" || field.kind === "consent") {
          continue;
        }
        const value = values[field.key];
        if (value === undefined) {
          continue;
        }
        if (Array.isArray(value)) {
          if (value.length > 0) {
            custom[field.key] = value;
          }
          continue;
        }
        const text = value.trim();
        if (text.length === 0) {
          continue;
        }
        if (CORE_FIELDS.has(field.key)) {
          assignCoreField(body, field.key, text);
        } else if (field.kind === "number") {
          const number = Number(text);
          if (!Number.isNaN(number)) {
            custom[field.key] = number;
          }
        } else {
          custom[field.key] = text;
        }
      }
      body.data = custom;

      const response = await client.inquiries.create(body);
      setStatus("success");
      onSubmitted?.(response.id);
      if (form.settings.redirect_url) {
        window.location.assign(form.settings.redirect_url);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Submission failed");
      setStatus("error");
    }
  }

  if (status === "success" && !form.settings.redirect_url) {
    return (
      <div className={className} role="status">
        {form.settings.success_message}
      </div>
    );
  }

  return (
    <form className={className} onSubmit={handleSubmit} noValidate>
      {title ? <h2>{title}</h2> : null}
      {description ? <p>{description}</p> : null}

      {form.fields.map((field) => (
        <FormField
          key={field.key}
          field={field}
          value={values[field.key]}
          files={files[field.key] ?? []}
          onChange={(value) => setValue(field.key, value)}
          onFilesChange={(selected) =>
            setFiles((current) => ({ ...current, [field.key]: selected }))
          }
        />
      ))}

      {(consentRequired || form.settings.consent_text) && (
        <label>
          <input
            type="checkbox"
            checked={consent}
            required={consentRequired}
            onChange={(event) => setConsent(event.target.checked)}
          />
          {consentText}
        </label>
      )}

      {error ? (
        <p role="alert" style={{ color: "#b91c1c" }}>
          {error}
        </p>
      ) : null}

      <button type="submit" disabled={status === "submitting"}>
        {status === "submitting" ? "Submitting…" : form.settings.submit_label}
      </button>
    </form>
  );
}

function FormField({
  field,
  value,
  files,
  onChange,
  onFilesChange,
}: {
  field: InquiryFormField;
  value: FieldValue | undefined;
  files: File[];
  onChange: (value: FieldValue) => void;
  onFilesChange: (files: File[]) => void;
}) {
  const id = `inquiry-${field.key}`;
  const help = field.help ? <small id={`${id}-help`}>{field.help}</small> : null;
  const describedBy = field.help ? `${id}-help` : undefined;

  if (field.kind === "hidden") {
    return <input type="hidden" name={field.key} value={stringValue(value)} />;
  }

  if (field.kind === "file") {
    const accept = field.file?.accept.join(",");
    return (
      <div>
        <label htmlFor={id}>
          {field.label}
          {field.required ? " *" : ""}
        </label>
        <input
          id={id}
          name={field.key}
          type="file"
          accept={accept}
          multiple={(field.file?.max_files ?? 1) > 1}
          required={field.required}
          aria-describedby={describedBy}
          onChange={(event) => onFilesChange(Array.from(event.target.files ?? []))}
        />
        {files.length > 0 ? (
          <small>
            {files.length} file{files.length === 1 ? "" : "s"} selected
          </small>
        ) : null}
        {help}
      </div>
    );
  }

  if (field.kind === "consent") {
    // Consent is rendered once at form level so every consent field shares the
    // same submitted boolean.
    return null;
  }

  if (field.kind === "checkbox") {
    const selected = Array.isArray(value) ? value : [];
    return (
      <fieldset>
        <legend>
          {field.label}
          {field.required ? " *" : ""}
        </legend>
        {field.options.map((option) => (
          <label key={option.value}>
            <input
              type="checkbox"
              name={field.key}
              value={option.value}
              checked={selected.includes(option.value)}
              onChange={(event) => {
                const next = event.target.checked
                  ? [...selected, option.value]
                  : selected.filter((item) => item !== option.value);
                onChange(next);
              }}
            />
            {option.label}
          </label>
        ))}
        {help}
      </fieldset>
    );
  }

  if (field.kind === "radio") {
    return (
      <fieldset>
        <legend>
          {field.label}
          {field.required ? " *" : ""}
        </legend>
        {field.options.map((option) => (
          <label key={option.value}>
            <input
              type="radio"
              name={field.key}
              value={option.value}
              checked={value === option.value}
              onChange={() => onChange(option.value)}
            />
            {option.label}
          </label>
        ))}
        {help}
      </fieldset>
    );
  }

  if (field.kind === "select") {
    return (
      <div>
        <label htmlFor={id}>
          {field.label}
          {field.required ? " *" : ""}
        </label>
        <select
          id={id}
          name={field.key}
          value={stringValue(value)}
          required={field.required}
          aria-describedby={describedBy}
          onChange={(event) => onChange(event.target.value)}
        >
          <option value="">{field.placeholder ?? "Select…"}</option>
          {field.options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
        {help}
      </div>
    );
  }

  if (field.kind === "textarea") {
    return (
      <div>
        <label htmlFor={id}>
          {field.label}
          {field.required ? " *" : ""}
        </label>
        <textarea
          id={id}
          name={field.key}
          value={stringValue(value)}
          placeholder={field.placeholder ?? undefined}
          required={field.required}
          minLength={field.validation.min_length ?? undefined}
          maxLength={field.validation.max_length ?? undefined}
          aria-describedby={describedBy}
          onChange={(event) => onChange(event.target.value)}
        />
        {help}
      </div>
    );
  }

  return (
    <div>
      <label htmlFor={id}>
        {field.label}
        {field.required ? " *" : ""}
      </label>
      <input
        id={id}
        name={field.key}
        type={inputType(field.kind)}
        value={stringValue(value)}
        placeholder={field.placeholder ?? undefined}
        required={field.required}
        min={field.validation.min ?? undefined}
        max={field.validation.max ?? undefined}
        minLength={field.validation.min_length ?? undefined}
        maxLength={field.validation.max_length ?? undefined}
        aria-describedby={describedBy}
        onChange={(event) => onChange(event.target.value)}
      />
      {help}
    </div>
  );
}

function initialValues(fields: InquiryFormField[]): Record<string, FieldValue> {
  const values: Record<string, FieldValue> = {};
  for (const field of fields) {
    if (field.kind === "checkbox") {
      values[field.key] = Array.isArray(field.default)
        ? field.default.filter((value): value is string => typeof value === "string")
        : [];
      continue;
    }
    if (typeof field.default === "string") {
      values[field.key] = field.default;
    } else if (typeof field.default === "number" || typeof field.default === "boolean") {
      values[field.key] = String(field.default);
    } else {
      values[field.key] = "";
    }
  }
  return values;
}

function stringValue(value: FieldValue | undefined): string {
  return Array.isArray(value) ? value.join(",") : (value ?? "");
}

function inputType(kind: InquiryFormField["kind"]): string {
  switch (kind) {
    case "email":
      return "email";
    case "phone":
      return "tel";
    case "number":
      return "number";
    case "date":
      return "date";
    default:
      return "text";
  }
}

function assignCoreField(body: CreateInquiryRequest, key: string, value: string) {
  switch (key) {
    case "name":
      body.name = value;
      break;
    case "email":
      body.email = value;
      break;
    case "phone":
      body.phone = value;
      break;
    case "subject":
      body.subject = value;
      break;
    case "message":
      body.message = value;
      break;
    default:
      break;
  }
}
