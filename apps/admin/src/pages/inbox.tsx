import { useCallback, useEffect, useMemo, useState } from "react";
import { Inbox, Paperclip, Search, X } from "lucide-react";
import { toast } from "sonner";
import type { Inquiry, InquiryFile, InquiryStatus } from "@fastcart/shared-types";

import { InquiryDetailSheet } from "@/components/inquiry-detail-sheet";
import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { InquiryStatusPill } from "@/components/status-pill";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAuth } from "@/lib/auth";
import { API_BASE_URL, errorMessage } from "@/lib/api";
import {
  demoInquiries,
  demoInquiryDetail,
  demoInquiryForms,
} from "@/lib/demo-inquiries";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  downloadInquiryFile,
  loadInquiries,
  loadInquiryDetail,
  updateInquiryStatus,
} from "@/lib/loaders";
import type { InquiriesPageView, InquiryDetailView } from "@/lib/models";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 20;

const STATUS_TABS: Array<{ value: InquiryStatus | "all"; label: string }> = [
  { value: "all", label: "All" },
  { value: "new", label: "New" },
  { value: "read", label: "Read" },
  { value: "replied", label: "Replied" },
  { value: "closed", label: "Closed" },
  { value: "spam", label: "Spam" },
];

function inquiryFileCount(inquiry: Inquiry): number {
  const record = inquiry as unknown as { files?: unknown; file_count?: unknown };
  if (Array.isArray(record.files)) return record.files.length;
  return typeof record.file_count === "number" ? record.file_count : 0;
}

function demoPage(
  items: Inquiry[],
  filters: { status: InquiryStatus | "all"; formId: string; query: string; from: string; to: string },
  offset: number,
): InquiriesPageView {
  const needle = filters.query.trim().toLowerCase();
  const from = filters.from.length > 0 ? new Date(`${filters.from}T00:00:00`).getTime() : null;
  const to = filters.to.length > 0 ? new Date(`${filters.to}T23:59:59`).getTime() : null;
  const filtered = items.filter((inquiry) => {
    if (filters.status !== "all" && inquiry.status !== filters.status) return false;
    if (filters.formId.length > 0 && inquiry.form_id !== filters.formId) return false;
    const at = inquiry.created_at * 1000;
    if (from !== null && at < from) return false;
    if (to !== null && at > to) return false;
    if (needle.length === 0) return true;
    return (
      (inquiry.name ?? "").toLowerCase().includes(needle) ||
      (inquiry.email ?? "").toLowerCase().includes(needle) ||
      (inquiry.subject ?? "").toLowerCase().includes(needle) ||
      (inquiry.message ?? "").toLowerCase().includes(needle)
    );
  });
  return {
    inquiries: filtered.slice(offset, offset + PAGE_SIZE),
    total: filtered.length,
    forms: demoInquiryForms(),
  };
}

export function InboxPage() {
  const { mode, client, store, dataKey } = useAuth();
  const [status, setStatus] = useState<InquiryStatus | "all">("all");
  const [formId, setFormId] = useState("");
  const [query, setQuery] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [page, setPage] = useState(1);
  const [demoItems, setDemoItems] = useState<Inquiry[]>(() => demoInquiries());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<InquiryDetailView | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

  useEffect(() => {
    setPage(1);
    setStatus("all");
    setFormId("");
    setQuery("");
    setFrom("");
    setTo("");
  }, [store?.id, dataKey]);

  const offset = (page - 1) * PAGE_SIZE;

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") {
        return Promise.resolve(
          demoPage(demoItems, { status, formId, query, from, to }, offset),
        );
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadInquiries(
        client,
        { status, formId, query, from, to, limit: PAGE_SIZE, offset },
        signal,
      );
    },
    [mode, client, demoItems, status, formId, query, from, to, offset],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    status,
    formId,
    query,
    from,
    to,
    offset,
    demoItems,
  ]);

  const total = data?.total ?? 0;
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const currentPage = Math.min(page, pages);

  const selectedInquiry = useMemo(() => {
    if (detail) return detail.inquiry;
    if (!selectedId) return null;
    return data?.inquiries.find((inquiry) => inquiry.id === selectedId) ?? null;
  }, [detail, selectedId, data]);

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      return;
    }
    if (mode === "demo") {
      const entry = demoInquiryDetail(selectedId);
      setDetail(
        entry
          ? { inquiry: entry.inquiry, revision: entry.revision, files: entry.files }
          : null,
      );
      return;
    }
    if (!client) return;
    const controller = new AbortController();
    setDetail(null);
    setDetailLoading(true);
    loadInquiryDetail(client, selectedId, controller.signal)
      .then((result) => setDetail(result))
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) toast.error(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setDetailLoading(false);
      });
    return () => controller.abort();
  }, [selectedId, mode, client]);

  const changeStatus = async (next: InquiryStatus) => {
    const inquiry = detail?.inquiry ?? selectedInquiry;
    if (!inquiry) return;
    try {
      if (mode === "demo") {
        setDemoItems((current) =>
          current.map((entry) =>
            entry.id === inquiry.id ? { ...entry, status: next } : entry,
          ),
        );
        setDetail((current) =>
          current ? { ...current, inquiry: { ...current.inquiry, status: next } } : current,
        );
        toast.success(`Marked as ${next}`);
      } else {
        if (!client) throw new Error("No store is selected.");
        const updated = await updateInquiryStatus(client, inquiry.id, next);
        setDetail((current) => (current ? { ...current, inquiry: updated } : current));
        reload();
        toast.success(`Marked as ${next}`);
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    }
  };

  const download = async (file: InquiryFile) => {
    if (mode === "demo") {
      toast.info("Attachments are simulated in the demo");
      return;
    }
    if (!client || !selectedInquiry) return;
    try {
      const url = `${API_BASE_URL}${client.inquiries.fileUrl(selectedInquiry.id, file.id)}`;
      await downloadInquiryFile(url, file.original_name ?? file.id);
    } catch (cause) {
      toast.error(errorMessage(cause));
    }
  };

  if (!data && mode !== "demo" && loading) return <TableSkeleton />;
  if (!data && mode !== "demo" && error) return <ErrorState message={error} onRetry={reload} />;

  const forms = data?.forms ?? [];
  const rows = data?.inquiries ?? [];

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Inbox"
        description="Every storefront inquiry in one place. Filter by form, status or date, then reply outside the admin."
        actions={
          <Button variant="outline" onClick={reload} disabled={loading}>
            <Inbox className="size-4" aria-hidden="true" />
            Refresh
          </Button>
        }
      />

      <div className="flex flex-col gap-3 xl:flex-row xl:items-center">
        <div className="relative w-full xl:w-[300px]">
          <Search
            className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
            aria-hidden="true"
          />
          <Input
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setPage(1);
            }}
            placeholder="Search name, email or message"
            className="pr-9 pl-9"
            aria-label="Search inquiries"
          />
          {query.length > 0 ? (
            <button
              type="button"
              onClick={() => {
                setQuery("");
                setPage(1);
              }}
              className="absolute top-1/2 right-2.5 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-faint hover:bg-accent hover:text-foreground"
              aria-label="Clear search"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>

        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto rounded-full border border-border bg-secondary p-1 no-scrollbar">
          {STATUS_TABS.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setStatus(option.value);
                setPage(1);
              }}
              aria-pressed={status === option.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center rounded-full px-3 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                status === option.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>

        <select
          value={formId}
          onChange={(event) => {
            setFormId(event.target.value);
            setPage(1);
          }}
          aria-label="Filter by form"
          className="h-10 cursor-pointer rounded-md border border-input bg-card px-3 text-[12.5px] text-foreground shadow-xs outline-none hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25"
        >
          <option value="">All forms</option>
          {forms.map((form) => (
            <option key={form.id} value={form.id}>
              {form.name}
            </option>
          ))}
        </select>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-2 text-[11.5px] text-muted-foreground">
          From
          <Input
            type="date"
            value={from}
            onChange={(event) => {
              setFrom(event.target.value);
              setPage(1);
            }}
            className="h-8 w-[150px]"
          />
        </label>
        <label className="flex items-center gap-2 text-[11.5px] text-muted-foreground">
          To
          <Input
            type="date"
            value={to}
            onChange={(event) => {
              setTo(event.target.value);
              setPage(1);
            }}
            className="h-8 w-[150px]"
          />
        </label>
        {from.length > 0 || to.length > 0 ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setFrom("");
              setTo("");
              setPage(1);
            }}
          >
            Clear dates
          </Button>
        ) : null}
      </div>

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">
              {status === "all" ? "All inquiries" : `${STATUS_TABS.find((tab) => tab.value === status)?.label ?? ""} inquiries`}
            </p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              {total.toLocaleString("en-US")} matched
            </p>
          </div>
        </div>

        {rows.length === 0 ? (
          <EmptyState
            title="No inquiries found"
            description="Try a different status, form or search term."
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="w-[90px] pl-6">Status</TableHead>
                <TableHead>Contact</TableHead>
                <TableHead className="hidden md:table-cell">Message</TableHead>
                <TableHead className="w-[80px] text-center">Files</TableHead>
                <TableHead className="w-[120px]">Received</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((inquiry) => (
                <TableRow
                  key={inquiry.id}
                  className="h-[62px] cursor-pointer hover:bg-accent/40"
                  onClick={() => setSelectedId(inquiry.id)}
                >
                  <TableCell className="pl-6">
                    <InquiryStatusPill status={inquiry.status} />
                  </TableCell>
                  <TableCell>
                    <p className="text-[12.5px] font-medium text-foreground">
                      {inquiry.name || "Anonymous"}
                    </p>
                    <p className="mt-0.5 text-[10.5px] text-faint">
                      {inquiry.email || inquiry.phone || "No contact details"}
                    </p>
                  </TableCell>
                  <TableCell className="hidden max-w-[340px] md:table-cell">
                    <p className="truncate text-[12px] text-ink-soft">
                      {inquiry.subject || inquiry.message || "—"}
                    </p>
                    {inquiry.subject && inquiry.message ? (
                      <p className="mt-0.5 truncate text-[10.5px] text-faint">
                        {inquiry.message}
                      </p>
                    ) : null}
                  </TableCell>
                  <TableCell className="text-center">
                    <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground">
                      <Paperclip className="size-3" aria-hidden="true" />
                      {inquiryFileCount(inquiry)}
                    </span>
                  </TableCell>
                  <TableCell className="text-[11.5px] text-muted-foreground">
                    {shortDate(fromUnixSeconds(inquiry.created_at))}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}

        <div className="flex flex-col gap-3 border-t border-border px-6 py-3.5 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[11.5px] text-muted-foreground">
            {total === 0
              ? "No inquiries to show"
              : `Showing ${offset + 1}–${offset + rows.length} of ${total.toLocaleString(
                  "en-US",
                )}`}
          </p>
          <Pagination page={currentPage} pages={pages} onChange={setPage} />
        </div>
      </Card>

      <InquiryDetailSheet
        open={selectedId !== null}
        detail={detail}
        loading={detailLoading}
        onClose={() => {
          setSelectedId(null);
          setDetail(null);
        }}
        onStatusChange={(next) => void changeStatus(next)}
        onDownload={(file) => void download(file)}
      />
    </div>
  );
}
