import { Plus, Trash2 } from "lucide-react";
import type { InquiryFieldKind, InquiryFormField } from "@fastcart/shared-types";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { FIELD_KINDS, FIELD_KIND_LABELS, defaultOptions, isChoiceKind } from "@/lib/inquiry-forms";

const selectClass =
  "h-9 w-full cursor-pointer appearance-none rounded-md border border-input bg-card px-3 text-[12.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25";

function numberValue(value: string): number | null {
  if (value.trim().length === 0) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function Section({ title, description }: { title: string; description?: string }) {
  return (
    <div className="pt-1">
      <p className="text-[11.5px] font-medium text-foreground">{title}</p>
      {description ? <p className="mt-0.5 text-[10.5px] text-faint">{description}</p> : null}
    </div>
  );
}

export function FieldSettings({
  field,
  onChange,
  onDelete,
}: {
  field: InquiryFormField | null;
  onChange: (field: InquiryFormField) => void;
  onDelete: () => void;
}) {
  if (!field) {
    return (
      <div className="rounded-xl border border-dashed border-border px-4 py-10 text-center">
        <p className="text-[12.5px] font-medium text-foreground">Nothing selected</p>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          Select a field to edit its label, validation and options.
        </p>
      </div>
    );
  }

  const update = (patch: Partial<InquiryFormField>) => {
    onChange({ ...field, ...patch });
  };

  const changeKind = (kind: InquiryFieldKind) => {
    const next: InquiryFormField = { ...field, kind };
    if (isChoiceKind(kind) && field.options.length === 0) next.options = defaultOptions();
    if (kind === "file" && !next.file) {
      next.file = {
        max_files: 5,
        max_bytes: 10 * 1024 * 1024,
        accept: ["image/jpeg", "image/png", "application/pdf"],
      };
    }
    onChange(next);
  };

  const updateOption = (index: number, patch: { value?: string; label?: string }) => {
    update({
      options: field.options.map((option, optionIndex) =>
        optionIndex === index ? { ...option, ...patch } : option,
      ),
    });
  };

  const file = field.file ?? {
    max_files: 5,
    max_bytes: 10 * 1024 * 1024,
    accept: ["image/jpeg", "image/png", "application/pdf"],
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-2">
        <p className="text-[12.5px] font-medium text-foreground">Field settings</p>
        <Button
          variant="ghost"
          size="icon-sm"
          className="text-destructive hover:text-destructive"
          aria-label={`Delete ${field.label}`}
          onClick={onDelete}
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>

      <div className="space-y-2">
        <Label htmlFor="field-label">Label</Label>
        <Input
          id="field-label"
          value={field.label}
          onChange={(event) => update({ label: event.target.value })}
          placeholder="Field label"
          className="h-9"
        />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <div className="space-y-2">
          <Label htmlFor="field-key">Key</Label>
          <Input
            id="field-key"
            value={field.key}
            onChange={(event) => update({ key: event.target.value })}
            placeholder="field_key"
            className="h-9 font-mono text-[12px]"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="field-kind">Type</Label>
          <select
            id="field-kind"
            value={field.kind}
            onChange={(event) => changeKind(event.target.value as InquiryFieldKind)}
            className={selectClass}
          >
            {FIELD_KINDS.map((kind) => (
              <option key={kind} value={kind}>
                {FIELD_KIND_LABELS[kind]}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-background px-3 py-2.5">
        <div>
          <p className="text-[12px] font-medium text-foreground">Required</p>
          <p className="mt-0.5 text-[10.5px] text-faint">Block submission until filled</p>
        </div>
        <Switch
          checked={field.required}
          onCheckedChange={(checked) => update({ required: checked })}
          aria-label="Required field"
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="field-placeholder">Placeholder</Label>
        <Input
          id="field-placeholder"
          value={field.placeholder ?? ""}
          onChange={(event) =>
            update({ placeholder: event.target.value.length > 0 ? event.target.value : null })
          }
          placeholder="Optional hint inside the input"
          className="h-9"
        />
      </div>

      <div className="space-y-2">
        <Label htmlFor="field-help">Help text</Label>
        <Input
          id="field-help"
          value={field.help ?? ""}
          onChange={(event) =>
            update({ help: event.target.value.length > 0 ? event.target.value : null })
          }
          placeholder="Shown under the input"
          className="h-9"
        />
      </div>

      {isChoiceKind(field.kind) ? (
        <>
          <Separator />
          <Section title="Options" description="Between 1 and 100 unique values." />
          <div className="space-y-1.5">
            {field.options.map((option, index) => (
              <div key={index} className="flex items-center gap-1.5">
                <Input
                  value={option.value}
                  onChange={(event) => updateOption(index, { value: event.target.value })}
                  aria-label={`Option ${index + 1} value`}
                  placeholder="value"
                  className="h-8 font-mono text-[11px]"
                />
                <Input
                  value={option.label}
                  onChange={(event) => updateOption(index, { label: event.target.value })}
                  aria-label={`Option ${index + 1} label`}
                  placeholder="Label"
                  className="h-8 text-[12px]"
                />
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Remove option ${index + 1}`}
                  onClick={() =>
                    update({ options: field.options.filter((_, entry) => entry !== index) })
                  }
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </div>
            ))}
          </div>
          <Button
            variant="outline"
            size="sm"
            className="w-full"
            onClick={() =>
              update({
                options: [
                  ...field.options,
                  {
                    value: `option_${field.options.length + 1}`,
                    label: `Option ${field.options.length + 1}`,
                  },
                ],
              })
            }
          >
            <Plus className="size-3.5" aria-hidden="true" />
            Add option
          </Button>
        </>
      ) : null}

      {field.kind === "file" ? (
        <>
          <Separator />
          <Section title="Uploads" description="Per-submission limits for this field." />
          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-2">
              <Label htmlFor="file-max-files">Max files</Label>
              <Input
                id="file-max-files"
                type="number"
                min={1}
                value={file.max_files}
                onChange={(event) =>
                  update({
                    file: { ...file, max_files: Math.max(1, Number(event.target.value) || 1) },
                  })
                }
                className="h-9"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="file-max-mb">Max size (MB)</Label>
              <Input
                id="file-max-mb"
                type="number"
                min={0.1}
                step={0.1}
                value={Math.round((file.max_bytes / (1024 * 1024)) * 10) / 10}
                onChange={(event) =>
                  update({
                    file: {
                      ...file,
                      max_bytes: Math.max(
                        1,
                        Math.round((Number(event.target.value) || 0.1) * 1024 * 1024),
                      ),
                    },
                  })
                }
                className="h-9"
              />
            </div>
          </div>
          <div className="space-y-2">
            <Label htmlFor="file-accept">Accepted types</Label>
            <Input
              id="file-accept"
              value={file.accept.join(", ")}
              onChange={(event) =>
                update({
                  file: {
                    ...file,
                    accept: event.target.value
                      .split(",")
                      .map((entry) => entry.trim())
                      .filter((entry) => entry.length > 0),
                  },
                })
              }
              placeholder="image/jpeg, image/png, application/pdf"
              className="h-9 font-mono text-[11px]"
            />
          </div>
        </>
      ) : null}

      <Separator />
      <Section title="Validation" description="Optional limits applied before submission." />
      <div className="grid grid-cols-2 gap-2">
        <div className="space-y-2">
          <Label htmlFor="validation-min-length">Min length</Label>
          <Input
            id="validation-min-length"
            type="number"
            min={0}
            value={field.validation.min_length ?? ""}
            onChange={(event) =>
              update({
                validation: {
                  ...field.validation,
                  min_length: numberValue(event.target.value),
                },
              })
            }
            className="h-9"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="validation-max-length">Max length</Label>
          <Input
            id="validation-max-length"
            type="number"
            min={1}
            value={field.validation.max_length ?? ""}
            onChange={(event) =>
              update({
                validation: {
                  ...field.validation,
                  max_length: numberValue(event.target.value),
                },
              })
            }
            className="h-9"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="validation-min">Min value</Label>
          <Input
            id="validation-min"
            type="number"
            value={field.validation.min ?? ""}
            onChange={(event) =>
              update({
                validation: {
                  ...field.validation,
                  min: numberValue(event.target.value),
                },
              })
            }
            className="h-9"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="validation-max">Max value</Label>
          <Input
            id="validation-max"
            type="number"
            value={field.validation.max ?? ""}
            onChange={(event) =>
              update({
                validation: {
                  ...field.validation,
                  max: numberValue(event.target.value),
                },
              })
            }
            className="h-9"
          />
        </div>
      </div>
    </div>
  );
}
