import { useCallback, useEffect, useState } from "react";
import { RefreshCw, RotateCcw } from "lucide-react";

import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
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
import { demoStoreReturns } from "@/lib/demo-commerce";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { PERMISSIONS } from "@/lib/permissions";
import {
  listStoreReturns,
  returnFilterOptions,
  returnStatusLabel,
  returnStatusTone,
  type StoreReturnPage,
  type StoreReturnRow,
} from "@/lib/returns";
import type { ReturnStatus } from "@fastcart/shared-types";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 20;

type StatusFilter = ReturnStatus | "all";

/**
 * Store-wide returns.
 *
 * Returns are created and driven from an order, but merchants triage them as
 * one queue across the store. The page is read-only: every row links back to
 * the order that owns it, where the existing after-sales actions live.
 */
export function ReturnsPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [page, setPage] = useState(1);
  const [demoRows] = useState<StoreReturnRow[]>(demoStoreReturns);

  const canReadOrders = can(PERMISSIONS.orders.read);

  useEffect(() => {
    setPage(1);
  }, [store?.id, mode, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<StoreReturnPage> => {
      if (mode === "demo") {
        const filtered = demoRows
          .filter((row) => statusFilter === "all" || row.status === statusFilter)
          .sort((left, right) => right.created_at - left.created_at);
        const offset = (page - 1) * PAGE_SIZE;
        return Promise.resolve({
          data: filtered.slice(offset, offset + PAGE_SIZE),
          pagination: { limit: PAGE_SIZE, offset, total: filtered.length },
        });
      }
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listStoreReturns(
        store.id,
        {
          limit: PAGE_SIZE,
          offset: (page - 1) * PAGE_SIZE,
          ...(statusFilter === "all" ? {} : { status: statusFilter }),
        },
        signal,
      );
    },
    [mode, store, demoRows, statusFilter, page],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    statusFilter,
    page,
  ]);

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
        title="Returns"
        description="Every return across the store, newest first. Open the order to request, receive or refund its return."
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
          aria-label="Filter returns by status"
          className="flex min-w-0 items-center gap-1 overflow-x-auto rounded-full border border-border bg-secondary p-1 no-scrollbar"
        >
          {returnFilterOptions().map((option) => (
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
          {total.toLocaleString("en-US")} {total === 1 ? "return" : "returns"}
          {mode === "demo" ? " · sample data" : ""}
        </p>
      </div>

      <Card className="overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="w-[110px] pl-6">Order</TableHead>
              <TableHead>Customer</TableHead>
              <TableHead className="w-[150px]">Status</TableHead>
              <TableHead className="hidden lg:table-cell">Reason</TableHead>
              <TableHead className="hidden w-[110px] text-right md:table-cell">Items</TableHead>
              <TableHead className="w-[130px] pr-6">Updated</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.id} className="h-[58px]">
                <TableCell className="pl-6">
                  {canReadOrders ? (
                    // The order drawer lives on the orders page; the order id
                    // in the query is what lets the integrator deep-link to a
                    // detail view once that route exists.
                    <a
                      href={`/orders?order=${encodeURIComponent(row.order_id)}`}
                      className="font-mono text-[12px] text-ink-soft underline-offset-4 hover:text-foreground hover:underline"
                      title="Open the order that owns this return"
                    >
                      #{row.order_display_number}
                    </a>
                  ) : (
                    <span className="font-mono text-[12px] text-ink-soft">
                      #{row.order_display_number}
                    </span>
                  )}
                </TableCell>
                <TableCell>
                  <p className="text-[12.5px] text-foreground">{row.customer_email}</p>
                  <p className="mt-1 font-mono text-[10.5px] text-faint">{row.id}</p>
                </TableCell>
                <TableCell>
                  <Badge variant={returnStatusTone(row.status)}>
                    {returnStatusLabel(row.status)}
                  </Badge>
                </TableCell>
                <TableCell className="hidden max-w-[300px] truncate text-[11.5px] text-muted-foreground lg:table-cell">
                  {row.reason ?? "—"}
                </TableCell>
                <TableCell className="hidden text-right text-[12.5px] text-foreground tnum md:table-cell">
                  {row.item_count}
                </TableCell>
                <TableCell className="pr-6 text-[11.5px] text-muted-foreground">
                  {shortDate(fromUnixSeconds(row.updated_at))}
                </TableCell>
              </TableRow>
            ))}
            {rows.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={6} className="p-0">
                  <EmptyState
                    title={
                      statusFilter === "all" ? "No returns yet" : "No returns with this status"
                    }
                    description={
                      statusFilter === "all"
                        ? "Returns appear here as soon as an operator records one on an order."
                        : "Clear the status filter to see every return in the store."
                    }
                    action={
                      statusFilter === "all" ? null : (
                        <Button variant="outline" onClick={() => setStatusFilter("all")}>
                          <RotateCcw className="size-4" aria-hidden="true" />
                          Show all returns
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
              ? "No returns to show"
              : `Showing ${(currentPage - 1) * PAGE_SIZE + 1}–${(currentPage - 1) * PAGE_SIZE + rows.length} of ${total.toLocaleString("en-US")}`}
          </p>
          <Pagination page={currentPage} pages={pages} onChange={setPage} />
        </div>
      </Card>
    </div>
  );
}
