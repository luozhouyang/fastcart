import type {
  InquiryFieldKind,
  InquiryFieldOption,
  InquiryFormField,
  InquiryFormRevision,
  InquiryFormSettings,
} from "@fastcart/shared-types";

export const FIELD_KINDS: InquiryFieldKind[] = [
  "text",
  "textarea",
  "email",
  "phone",
  "number",
  "select",
  "radio",
  "checkbox",
  "date",
  "file",
  "hidden",
  "consent",
];

export const FIELD_KIND_LABELS: Record<InquiryFieldKind, string> = {
  text: "Text",
  textarea: "Long text",
  email: "Email",
  phone: "Phone",
  number: "Number",
  select: "Select",
  radio: "Radio",
  checkbox: "Checkbox",
  date: "Date",
  file: "File upload",
  hidden: "Hidden",
  consent: "Consent",
};

export const CHOICE_KINDS: InquiryFieldKind[] = ["select", "radio", "checkbox"];

/** Field keys that map to dedicated columns on an inquiry. */
export const RESERVED_KEYS = ["name", "email", "phone", "subject", "message"] as const;

export const KEY_PATTERN = /^[a-z][a-z0-9_]{0,63}$/;

const RESERVED_KEY_KINDS: Record<(typeof RESERVED_KEYS)[number], InquiryFieldKind[]> = {
  name: ["text", "hidden"],
  email: ["email", "text", "hidden"],
  phone: ["phone", "text", "hidden"],
  subject: ["text", "hidden"],
  message: ["textarea", "text", "hidden"],
};

export function defaultFormSettings(): InquiryFormSettings {
  return {
    submit_label: "Submit",
    success_message: "Thanks — we have received your message and will get back to you soon.",
    redirect_url: null,
    consent_text: null,
    consent_required: false,
    allow_attachments: true,
    notification_fields: [],
  };
}

export function isChoiceKind(kind: InquiryFieldKind): boolean {
  return CHOICE_KINDS.includes(kind);
}

export function defaultOptions(): InquiryFieldOption[] {
  return [
    { value: "option_1", label: "Option 1" },
    { value: "option_2", label: "Option 2" },
  ];
}

/** Creates a new field with a key that starts unique against the supplied fields. */
export function createField(
  kind: InquiryFieldKind,
  existing: InquiryFormField[],
): InquiryFormField {
  const used = new Set(existing.map((field) => field.key));
  let suffix = existing.length + 1;
  let key = `${kind}_${suffix}`;
  while (used.has(key)) {
    suffix += 1;
    key = `${kind}_${suffix}`;
  }

  const field: InquiryFormField = {
    key,
    kind,
    label: FIELD_KIND_LABELS[kind],
    required: false,
    placeholder: null,
    help: null,
    default: null,
    options: [],
    validation: {},
    file: null,
  };

  if (isChoiceKind(kind)) {
    field.options = defaultOptions();
  }

  if (kind === "file") {
    field.label = "Attachment";
    field.file = {
      max_files: 5,
      max_bytes: 10 * 1024 * 1024,
      accept: ["image/jpeg", "image/png", "application/pdf"],
    };
  }

  if (kind === "consent") {
    field.label = "Consent";
    field.required = true;
  }

  if (kind === "email" || kind === "phone") {
    field.required = true;
  }

  return field;
}

/** Picks the draft revision if one exists, otherwise the newest revision. */
export function revisionForEditing(revisions: InquiryFormRevision[]): InquiryFormRevision | null {
  const draft = revisions.find(
    (revision) => revision.published_at === null || revision.published_at === undefined,
  );
  if (draft) return draft;
  return [...revisions].sort((left, right) => right.version - left.version)[0] ?? null;
}

/**
 * Validates a form draft before saving or publishing.
 *
 * Mirrors the server rules: key shape, uniqueness, options, contact coverage,
 * attachment limits and notification columns.
 */
export function validateInquiryDraft(
  fields: InquiryFormField[],
  settings: InquiryFormSettings,
): string[] {
  const errors: string[] = [];
  if (fields.length === 0) {
    errors.push("Add at least one field before saving the form.");
  }

  const seen = new Set<string>();
  for (const [index, field] of fields.entries()) {
    const label = field.label.trim().length > 0 ? field.label.trim() : `Field ${index + 1}`;

    if (field.label.trim().length === 0) {
      errors.push(`Field ${index + 1}: add a label.`);
    }

    if (!KEY_PATTERN.test(field.key)) {
      errors.push(
        `${label}: key must start with a lowercase letter and use only lowercase letters, numbers and underscores (max 64 characters).`,
      );
    }

    if (seen.has(field.key)) {
      errors.push(`${label}: key "${field.key}" is used by another field.`);
    }
    seen.add(field.key);

    if ((RESERVED_KEYS as readonly string[]).includes(field.key)) {
      const allowed = RESERVED_KEY_KINDS[field.key as (typeof RESERVED_KEYS)[number]];
      if (allowed && !allowed.includes(field.kind)) {
        errors.push(
          `${label}: "${field.key}" is a reserved key and must use one of these field types: ${allowed
            .map((kind) => FIELD_KIND_LABELS[kind])
            .join(", ")}.`,
        );
      }
    }

    if (isChoiceKind(field.kind)) {
      if (field.options.length === 0 || field.options.length > 100) {
        errors.push(`${label}: add between 1 and 100 options.`);
      }
      const values = new Set<string>();
      for (const option of field.options) {
        if (option.value.trim().length === 0) {
          errors.push(`${label}: every option needs a value.`);
          continue;
        }
        if (values.has(option.value)) {
          errors.push(`${label}: option value "${option.value}" is duplicated.`);
        }
        values.add(option.value);
      }
    }

    if (field.kind === "file") {
      if (!field.file) {
        errors.push(`${label}: configure the upload limits.`);
      } else {
        if (field.file.max_files < 1) errors.push(`${label}: allow at least one file.`);
        if (field.file.max_bytes < 1) errors.push(`${label}: the size limit must be positive.`);
        if (field.file.accept.length === 0) {
          errors.push(`${label}: list at least one accepted content type.`);
        }
      }
    }

    const { min_length, max_length, min, max } = field.validation;
    if (min_length != null && min_length < 0) {
      errors.push(`${label}: minimum length cannot be negative.`);
    }
    if (max_length != null && max_length < 1) {
      errors.push(`${label}: maximum length must be at least 1.`);
    }
    if (min_length != null && max_length != null && min_length > max_length) {
      errors.push(`${label}: minimum length cannot exceed maximum length.`);
    }
    if (min != null && max != null && min > max) {
      errors.push(`${label}: minimum value cannot exceed maximum value.`);
    }
  }

  const hasContactField = fields.some(
    (field) =>
      field.kind !== "hidden" &&
      (field.kind === "email" || field.kind === "phone" || field.key === "message"),
  );
  if (!hasContactField) {
    errors.push("Add a visible email, phone or message field so you can reply.");
  }

  if (settings.notification_fields.length > 6) {
    errors.push("Choose at most 6 fields for notifications.");
  }
  for (const key of settings.notification_fields) {
    if (!seen.has(key)) {
      errors.push(`Notification field "${key}" is not part of this form.`);
    }
  }

  if (settings.submit_label.trim().length === 0) {
    errors.push("Add a submit button label.");
  }
  if (settings.success_message.trim().length === 0) {
    errors.push("Add a success message.");
  }

  return errors;
}

/** Stable string used for the unsaved-changes comparison. */
export function draftFingerprint(
  fields: InquiryFormField[],
  settings: InquiryFormSettings,
): string {
  return JSON.stringify({ fields, settings });
}

export function fieldValueLabel(value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value === "string" || typeof value === "number") return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

export function fieldOptionLabel(field: InquiryFormField, value: unknown): string {
  if (Array.isArray(value)) {
    return value.map((entry) => fieldOptionLabel(field, entry)).join(", ");
  }
  const option = field.options.find((entry) => entry.value === String(value));
  return option?.label ?? fieldValueLabel(value);
}
