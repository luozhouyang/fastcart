import { describe, expect, it } from "vitest";
import type { InquiryFormField, InquiryFormSettings } from "@fastcart/shared-types";

import {
  createField,
  defaultFormSettings,
  draftFingerprint,
  revisionForEditing,
  validateInquiryDraft,
} from "@/lib/inquiry-forms";

function textField(overrides: Partial<InquiryFormField> = {}): InquiryFormField {
  return {
    key: "name",
    kind: "text",
    label: "Name",
    required: true,
    placeholder: null,
    help: null,
    default: null,
    options: [],
    validation: {},
    file: null,
    ...overrides,
  };
}

function contactFields(): InquiryFormField[] {
  return [
    textField(),
    textField({ key: "email", kind: "email", label: "Email" }),
    textField({ key: "message", kind: "textarea", label: "Message" }),
  ];
}

describe("validateInquiryDraft", () => {
  it("accepts a draft with a contact field and valid keys", () => {
    const settings: InquiryFormSettings = defaultFormSettings();
    expect(validateInquiryDraft(contactFields(), settings)).toEqual([]);
  });

  it("rejects invalid and duplicate keys", () => {
    const fields = [
      textField({ key: "Name", label: "Bad key" }),
      textField({ key: "email", kind: "email", label: "Email" }),
      textField({ key: "email", kind: "email", label: "Duplicate email" }),
    ];
    const errors = validateInquiryDraft(fields, defaultFormSettings());
    expect(errors.some((error) => error.includes("key must start with a lowercase letter"))).toBe(
      true,
    );
    expect(errors.some((error) => error.includes('key "email" is used by another field'))).toBe(
      true,
    );
  });

  it("requires options for choice fields and a visible contact field", () => {
    const fields = [
      textField({ key: "name", label: "Name" }),
      textField({
        key: "budget",
        kind: "select",
        label: "Budget",
        options: [{ value: "", label: "Empty" }],
      }),
    ];
    const errors = validateInquiryDraft(fields, defaultFormSettings());
    expect(errors.some((error) => error.includes("every option needs a value"))).toBe(true);
    expect(
      errors.some((error) => error.includes("Add a visible email, phone or message field")),
    ).toBe(true);
  });

  it("validates notification fields against the current revision", () => {
    const settings: InquiryFormSettings = {
      ...defaultFormSettings(),
      notification_fields: ["missing"],
    };
    const errors = validateInquiryDraft(contactFields(), settings);
    expect(errors.some((error) => error.includes('"missing" is not part of this form'))).toBe(true);
  });
});

describe("revisionForEditing", () => {
  it("prefers the unpublished draft revision", () => {
    const draft = revisionForEditing([
      {
        id: "rev-2",
        form_id: "form-1",
        version: 2,
        fields: [],
        settings: defaultFormSettings(),
        created_at: 2,
        published_at: null,
      },
      {
        id: "rev-1",
        form_id: "form-1",
        version: 1,
        fields: [],
        settings: defaultFormSettings(),
        created_at: 1,
        published_at: 10,
      },
    ]);
    expect(draft?.version).toBe(2);
  });
});

describe("field helpers", () => {
  it("creates unique keys and default file limits", () => {
    const existing = [createField("file", [])];
    const next = createField("file", existing);
    expect(next.key).toBe("file_2");
    expect(next.file?.max_files).toBe(5);
    expect(draftFingerprint([next], defaultFormSettings())).toContain("file_2");
  });
});
