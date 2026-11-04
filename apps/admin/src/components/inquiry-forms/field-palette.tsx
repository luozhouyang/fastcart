import {
  AlignLeft,
  Calendar,
  CheckSquare,
  Circle,
  EyeOff,
  Hash,
  List,
  Mail,
  Paperclip,
  Phone,
  ShieldCheck,
  Type,
} from "lucide-react";
import type { InquiryFieldKind } from "@fastcart/shared-types";

import { FIELD_KINDS, FIELD_KIND_LABELS } from "@/lib/inquiry-forms";

const ICONS: Record<InquiryFieldKind, typeof Type> = {
  text: Type,
  textarea: AlignLeft,
  email: Mail,
  phone: Phone,
  number: Hash,
  select: List,
  radio: Circle,
  checkbox: CheckSquare,
  date: Calendar,
  file: Paperclip,
  hidden: EyeOff,
  consent: ShieldCheck,
};

export function FieldPalette({ onAdd }: { onAdd: (kind: InquiryFieldKind) => void }) {
  return (
    <div className="space-y-1.5">
      <p className="px-1 pb-1 text-[11px] font-medium text-muted-foreground">Add a field</p>
      <div className="grid grid-cols-2 gap-1.5 xl:grid-cols-1">
        {FIELD_KINDS.map((kind) => {
          const Icon = ICONS[kind];
          return (
            <button
              key={kind}
              type="button"
              onClick={() => onAdd(kind)}
              className="flex h-8 cursor-pointer items-center gap-2 rounded-lg border border-transparent px-2.5 text-left text-[12px] text-ink-soft transition-colors hover:border-border hover:bg-card hover:text-foreground"
            >
              <Icon className="size-3.5 shrink-0 text-faint" aria-hidden="true" />
              <span className="truncate">{FIELD_KIND_LABELS[kind]}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
