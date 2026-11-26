import { useCallback, useEffect, useState } from "react";
import { ChevronRight, RefreshCw } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
import { PaymentDetailSheet } from "@/components/payment-detail-sheet";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useAuth } from "@/lib/auth";
import { demoPayments } from "@/lib/demo-commerce";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  listPayments,
  paymentFilterOptions,
  paymentMethodLabel,
  paymentStatusLabel,
  paymentStatusTone,
  type AdminPayment,
  type AdminPaymentStatus,
} from "@/lib/payments";
import { formatMinorAmount } from "@/lib/pricing";
import { useAsyncData } from "@/lib/use-async-data";
import type { PageResult } from "@fastcart/shared-types";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 15;

type StatusFilter = AdminPaymentStatus | "all";

export function PaymentsPage() {
  const { mode, store, dataKey } = useAuth();
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [page, setPage] = useState(1);
  const [detail, setDetail] = useState<AdminPayment | null>(null);
  const [demoList] = useState<AdminPayment[]>(demoPayments);

  const loader = useCallback(
    (signal: AbortSignal): Promise<PageResult<AdminPayment>> => {
      if (mode === "demo") {
        const filtered = demoList
          .filter((payment) => statusFilter === "all" || payment.status === statusFilter)
          .sort((left, right) => right.updated_at - left.updated_at);
        const offset = (page - 1) * PAGE_SIZE;
        return Promise.resolve({
          data: filtered.slice(offset, offset + PAGE_SIZE),
          pagination: { limit: PAGE_SIZE, offset, total: filtered.length },
        });
      }
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listPayments(
        store.id,
        {
          limit: PAGE_SIZE,
          offset: (page - 1) * PAGE_SIZE,
          ...(statusFilter === "all" ? {} : { status: statusFilter }),
        },
        signal,
      );
    },
    [mode, store, demoList, statusFilter, page],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    statusFilter,
    page,
  ]);

  useEffect(() => {
    setPage(1);
    setDetail(null);
  }, [store?.id, dataKey]);

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const rows = data.data;
  const total = data.pagination.total;
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const currentPage = Math.min(page, pages);

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Payments"
        description="Review captured payments, refund them, and manage payment sessions that still need an action."
        actions={
          <Button variant="outline" onClick={reload} disabled={loading}>
            <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            Refresh
          </Button>
        }
      />

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div
          role="group"
          aria-label="Filter payments by status"
          className="flex min-w-0 items-center gap-1 overflow-x-auto rounded-full border border-border bg-secondary p-1 no-scrollbar"
        >
          {paymentFilterOptions().map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setStatusFilter(option.value);
                setPage(1);
              }}
              aria-pressed={statusFilter === option.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center rounded-full px-3 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                statusFilter === option.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
        <p className="text-[11.5px] text-muted-foreground">
          {total.toLocaleString("en-US")} {total === 1 ? "payment" : "payments"}
          {mode === "demo" ? " · sample data" : ""}
        </p>
      </div>

      <Card className="overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Amount</TableHead>
              <TableHead className="w-[150px]">Status</TableHead>
              <TableHead className="hidden w-[160px] md:table-cell">Order</TableHead>
              <TableHead className="hidden w-[130px] lg:table-cell">Method</TableHead>
              <TableHead className="hidden w-[130px] xl:table-cell">Created</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Open" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((payment) => (
              <TableRow key={payment.id}>
                <TableCell className="pl-6">
                  <button
                    type="button"
                    onClick={() => setDetail(payment)}
                    className="cursor-pointer text-left text-[13px] font-medium text-foreground underline-offset-4 hover:underline tnum"
                    aria-label={`Open payment ${payment.id}`}
                  >
                    {formatMinorAmount(payment.amount, payment.currency_code)}
                  </button>
                  <p className="mt-1 font-mono text-[10.5px] text-faint">{payment.id}</p>
                </TableCell>
                <TableCell>
                  <Badge variant={paymentStatusTone(payment.status)}>
                    {paymentStatusLabel(payment.status)}
                  </Badge>
                </TableCell>
                <TableCell className="hidden font-mono text-[11.5px] text-ink-soft md:table-cell">
                  {payment.order_id ?? "—"}
                </TableCell>
                <TableCell className="hidden lg:table-cell">
                  <Badge variant="outline">{paymentMethodLabel(payment)}</Badge>
                </TableCell>
                <TableCell className="hidden text-muted-foreground xl:table-cell">
                  {shortDate(fromUnixSeconds(payment.created_at))}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <ChevronRight className="ml-auto size-4 text-faint" aria-hidden="true" />
                </TableCell>
              </TableRow>
            ))}
            {rows.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={6} className="p-0">
                  <EmptyState
                    title={
                      statusFilter === "all" ? "No payments yet" : "No payments with this status"
                    }
                    description={
                      statusFilter === "all"
                        ? "Payments appear here as soon as a checkout session is captured or a provider confirms a charge."
                        : "Clear the status filter to see every recorded payment."
                    }
                    action={
                      statusFilter === "all" ? null : (
                        <Button variant="outline" onClick={() => setStatusFilter("all")}>
                          Show all payments
                        </Button>
                      )
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>

        <div className="flex flex-col gap-3 border-t border-border px-6 py-3.5 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[11.5px] text-muted-foreground">
            {total === 0
              ? "No payments to show"
              : `Showing ${(currentPage - 1) * PAGE_SIZE + 1}–${(currentPage - 1) * PAGE_SIZE + rows.length} of ${total.toLocaleString("en-US")}`}
          </p>
          <Pagination page={currentPage} pages={pages} onChange={setPage} />
        </div>
      </Card>

      <PaymentDetailSheet
        payment={detail}
        onOpenChange={(open) => {
          if (!open) setDetail(null);
        }}
      />
    </div>
  );
}
