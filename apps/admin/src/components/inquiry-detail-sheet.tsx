import { Download, ExternalLink, Loader2, Paperclip } from "lucide-react";
import type { Inquiry, InquiryFile, InquiryStatus } from "@fastcart/shared-types";

import { InquiryStatusPill } from "@/components/status-pill";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import type { InquiryDetailView } from "@/lib/models";
import { fieldOptionLabel, fieldValueLabel } from "@/lib/inquiry-forms";
import { fromUnixSeconds, shortDate, timeOfDay } from "@/lib/format";

const STATUS_ACTIONS: InquiryStatus[] = ["new", "read", "replied", "closed", "spam"];

function coreValue(inquiry: Inquiry, key: string): unknown {
  if (
    key === "name" ||
    key === "email" ||
    key === "phone" ||
    key === "subject" ||
    key === "message"
  ) {
    return inquiry[key];
  }
  return inquiry.data[key];
}

function fileSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${Math.round((bytes / (1024 * 1024)) * 10) / 10} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="text-[10.5px] font-medium text-faint">{label}</p>
      <p className="mt-0.5 break-words text-[12px] text-foreground">{value}</p>
    </div>
  );
}

export function InquiryDetailSheet({
  open,
  detail,
  loading,
  onClose,
  onStatusChange,
  onDownload,
}: {
  open: boolean;
  detail: InquiryDetailView | null;
  loading: boolean;
  onClose: () => void;
  onStatusChange: (status: InquiryStatus) => void;
  onDownload: (file: InquiryFile) => void;
}) {
  const inquiry = detail?.inquiry ?? null;
  const revision = detail?.revision ?? null;
  const files = detail?.files ?? [];

  return (
    <Sheet open={open} onOpenChange={(next) => !next && onClose()}>
      <SheetContent side="right" className="w-full p-0 sm:max-w-lg">
        <SheetHeader>
          <div className="flex items-center gap-2">
            {inquiry ? <InquiryStatusPill status={inquiry.status} /> : null}
            <SheetTitle className="min-w-0 flex-1 truncate">
              {inquiry?.subject || inquiry?.name || "Inquiry"}
            </SheetTitle>
          </div>
          <SheetDescription>
            {inquiry
              ? `Received ${shortDate(fromUnixSeconds(inquiry.created_at))} at ${timeOfDay(
                  fromUnixSeconds(inquiry.created_at),
                )}`
              : "Loading the inquiry…"}
          </SheetDescription>
        </SheetHeader>

        {loading ? (
          <div className="flex flex-1 items-center justify-center py-16">
            <Loader2 className="size-5 animate-spin text-faint" aria-hidden="true" />
          </div>
        ) : inquiry ? (
          <>
            <div className="flex-1 space-y-5 overflow-y-auto p-6">
              <div className="flex flex-wrap gap-1.5">
                {STATUS_ACTIONS.map((status) => (
                  <Button
                    key={status}
                    variant={inquiry.status === status ? "default" : "outline"}
                    size="sm"
                    onClick={() => onStatusChange(status)}
                  >
                    {status.charAt(0).toUpperCase() + status.slice(1)}
                  </Button>
                ))}
              </div>

              <div className="grid grid-cols-2 gap-3">
                <DetailRow label="Name" value={inquiry.name ?? "—"} />
                <DetailRow label="Email" value={inquiry.email ?? "—"} />
                <DetailRow label="Phone" value={inquiry.phone ?? "—"} />
                <DetailRow label="Source" value={inquiry.source} />
                <DetailRow label="Form version" value={`v${inquiry.form_version}`} />
                <DetailRow label="Locale" value={inquiry.locale ?? "—"} />
              </div>

              {inquiry.page_url ? (
                <a
                  href={inquiry.page_url}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-1.5 text-[11.5px] text-brand-ink underline-offset-4 hover:underline"
                >
                  <ExternalLink className="size-3.5" aria-hidden="true" />
                  View submitted page
                </a>
              ) : null}

              <Separator />

              <div>
                <p className="text-[11.5px] font-medium text-foreground">Submission</p>
                <div className="mt-2 space-y-3">
                  {revision ? (
                    revision.fields
                      .filter((field) => field.kind !== "hidden")
                      .map((field) => {
                        const raw = coreValue(inquiry, field.key);
                        const value =
                          field.options.length > 0
                            ? fieldOptionLabel(field, raw)
                            : fieldValueLabel(raw);
                        return (
                          <div key={field.key}>
                            <p className="text-[10.5px] font-medium text-faint">{field.label}</p>
                            <p className="mt-0.5 whitespace-pre-wrap break-words text-[12.5px] text-foreground">
                              {value}
                            </p>
                          </div>
                        );
                      })
                  ) : (
                    <>
                      {(["name", "email", "phone", "subject", "message"] as const).map((key) => {
                        const value = inquiry[key];
                        if (value === null || value === undefined || value === "") return null;
                        return (
                          <div key={key}>
                            <p className="text-[10.5px] font-medium text-faint capitalize">{key}</p>
                            <p className="mt-0.5 whitespace-pre-wrap break-words text-[12.5px] text-foreground">
                              {value}
                            </p>
                          </div>
                        );
                      })}
                      {Object.entries(inquiry.data).map(([key, value]) => (
                        <div key={key}>
                          <p className="mt-0.5 text-[10.5px] font-medium text-faint">{key}</p>
                          <p className="mt-0.5 whitespace-pre-wrap break-words text-[12.5px] text-foreground">
                            {fieldValueLabel(value)}
                          </p>
                        </div>
                      ))}
                    </>
                  )}
                </div>
              </div>

              {files.length > 0 ? (
                <>
                  <Separator />
                  <div>
                    <p className="flex items-center gap-1.5 text-[11.5px] font-medium text-foreground">
                      <Paperclip className="size-3.5" aria-hidden="true" />
                      Attachments
                    </p>
                    <ul className="mt-2 space-y-1.5">
                      {files.map((file) => (
                        <li
                          key={file.id}
                          className="flex items-center gap-2 rounded-lg border border-border bg-background px-3 py-2"
                        >
                          <div className="min-w-0 flex-1">
                            <p className="truncate text-[12px] text-foreground">
                              {file.original_name ?? file.field_key}
                            </p>
                            <p className="mt-0.5 text-[10.5px] text-faint">
                              {file.content_type} · {fileSize(file.size_bytes)}
                            </p>
                          </div>
                          <Button variant="outline" size="sm" onClick={() => onDownload(file)}>
                            <Download className="size-3.5" aria-hidden="true" />
                            Download
                          </Button>
                        </li>
                      ))}
                    </ul>
                  </div>
                </>
              ) : null}
            </div>
          </>
        ) : (
          <div className="flex flex-1 items-center justify-center px-6 py-16 text-center">
            <p className="text-[12.5px] text-muted-foreground">This inquiry could not be loaded.</p>
          </div>
        )}
      </SheetContent>
    </Sheet>
  );
}
