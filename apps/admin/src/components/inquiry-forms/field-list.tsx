import { ArrowDown, ArrowUp, GripVertical, Trash2 } from "lucide-react";
import type { InquiryFormField } from "@fastcart/shared-types";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { FIELD_KIND_LABELS } from "@/lib/inquiry-forms";
import { cn } from "@/lib/utils";

export function FieldList({
  fields,
  selectedIndex,
  onSelect,
  onMove,
  onDelete,
}: {
  fields: InquiryFormField[];
  selectedIndex: number | null;
  onSelect: (index: number) => void;
  onMove: (index: number, direction: -1 | 1) => void;
  onDelete: (index: number) => void;
}) {
  if (fields.length === 0) {
    return (
      <div className="rounded-xl border border-dashed border-border px-4 py-10 text-center">
        <p className="text-[12.5px] font-medium text-foreground">No fields yet</p>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          Add a field from the palette to start building the form.
        </p>
      </div>
    );
  }

  return (
    <ul className="space-y-1.5" aria-label="Form fields">
      {fields.map((field, index) => {
        const active = selectedIndex === index;
        return (
          <li key={`${field.key}-${index}`}>
            <div
              className={cn(
                "group flex items-center gap-2 rounded-xl border px-2.5 py-2 transition-colors",
                active
                  ? "border-brand/35 bg-brand-soft/50"
                  : "border-border bg-card hover:border-line-strong",
              )}
            >
              <button
                type="button"
                onClick={() => onSelect(index)}
                className="flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 py-0.5 text-left"
              >
                <GripVertical className="size-3.5 shrink-0 text-faint" aria-hidden="true" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate text-[12.5px] font-medium text-foreground">
                      {field.label || "Untitled field"}
                    </span>
                    {field.required ? (
                      <Badge variant="outline" className="px-1.5 py-0 text-[10px]">
                        Required
                      </Badge>
                    ) : null}
                  </span>
                  <span className="mt-0.5 flex items-center gap-1.5 text-[10.5px] text-faint">
                    <span>{FIELD_KIND_LABELS[field.kind]}</span>
                    <span aria-hidden="true">·</span>
                    <span className="truncate font-mono">{field.key}</span>
                  </span>
                </span>
              </button>
              <div className="flex items-center gap-0.5 opacity-70 transition-opacity group-hover:opacity-100">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Move ${field.label} up`}
                  disabled={index === 0}
                  onClick={() => onMove(index, -1)}
                >
                  <ArrowUp className="size-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Move ${field.label} down`}
                  disabled={index === fields.length - 1}
                  onClick={() => onMove(index, 1)}
                >
                  <ArrowDown className="size-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Delete ${field.label}`}
                  className="text-destructive hover:text-destructive"
                  onClick={() => onDelete(index)}
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </div>
            </div>
          </li>
        );
      })}
    </ul>
  );
}
