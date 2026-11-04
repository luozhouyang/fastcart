import { useCallback, useDeferredValue, useEffect, useState } from "react";
import {
  CalendarDays,
  Check,
  ChevronDown,
  ChevronRight,
  Download,
  RefreshCw,
  Search,
  X,
} from "lucide-react";
import { toast } from "sonner";

import { OrderDrawer } from "@/components/order-drawer";
import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
import { ErrorState, TableSkeleton } from "@/components/states";
import {
  FulfillmentPill,
  OrderPill,
  PaymentPill,
  StatusRail,
  orderRailTone,
} from "@/components/status-pill";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
import { demoOrdersPage } from "@/lib/demo";
import {
  count as formatCount,
  downloadCsv,
  initials,
  money,
  monthDay,
  timeOfDay,
} from "@/lib/format";
import { fulfillOrder, loadOrders, refundOrder, type OrderSort } from "@/lib/loaders";
import {
  ORDER_STATUS_LABELS,
  type OrderStatusView,
  type OrderView,
  type RevenueRange,
} from "@/lib/models";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 10;

const STATUS_TABS: Array<{ value: OrderStatusView | "all"; label: string }> = [
  { value: "all", label: "All" },
  { value: "pending", label: "Pending" },
  { value: "confirmed", label: "Confirmed" },
  { value: "processing", label: "Processing" },
  { value: "fulfilled", label: "Fulfilled" },
  { value: "completed", label: "Completed" },
  { value: "canceled", label: "Canceled" },
];

const RANGE_OPTIONS: Array<{ value: "all" | RevenueRange; label: string }> = [
  { value: "all", label: "All time" },
  { value: "7d", label: "Last 7 days" },
  { value: "30d", label: "Last 30 days" },
  { value: "90d", label: "Last 90 days" },
];

const SORT_OPTIONS: Array<{ value: OrderSort; label: string }> = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "total", label: "Highest total" },
];

export function OrdersPage() {
  const { mode, client, store, dataKey } = useAuth();
  const [status, setStatus] = useState<OrderStatusView | "all">("all");
  const [page, setPage] = useState(1);
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [range, setRange] = useState<"all" | RevenueRange>("all");
  const [sort, setSort] = useState<OrderSort>("newest");
  const [selected, setSelected] = useState<OrderView | null>(null);
  const [patched, setPatched] = useState<Record<string, OrderView>>({});

  useEffect(() => {
    setPatched({});
    setSelected(null);
    setPage(1);
  }, [store?.id, mode, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") {
        return Promise.resolve(
          demoOrdersPage({
            status,
            page,
            pageSize: PAGE_SIZE,
            query: deferredQuery,
            range,
            sort,
          }),
        );
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadOrders(
        client,
        { status, page, pageSize: PAGE_SIZE, query: deferredQuery, range, sort },
        signal,
      );
    },
    [mode, client, status, page, deferredQuery, range, sort],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    status,
    page,
    deferredQuery,
    range,
    sort,
  ]);

  const rows = (data?.orders ?? []).map((order) => patched[order.id] ?? order);
  const selectedOrder = selected ? (patched[selected.id] ?? selected) : null;
  const total = data?.total ?? 0;
  const pages =
    mode === "demo"
      ? Math.max(1, Math.ceil((data?.loaded ?? rows.length) / PAGE_SIZE))
      : Math.max(1, Math.ceil(total / PAGE_SIZE));
  const start = rows.length === 0 ? 0 : (page - 1) * PAGE_SIZE + 1;
  const end = start === 0 ? 0 : start + rows.length - 1;

  const handleFulfill = async (order: OrderView, tracking: string) => {
    if (mode === "demo") {
      const updated: OrderView = {
        ...order,
        status: "fulfilled",
        fulfillmentStatus: "fulfilled",
        canFulfill: false,
        history: [
          ...order.history,
          { label: "Fulfilled", at: new Date().toISOString(), tone: "brand" },
        ],
      };
      setPatched((current) => ({ ...current, [order.id]: updated }));
      setSelected(updated);
      toast.success(`Order #${order.number} marked as fulfilled`);
      return;
    }
    if (!client) throw new Error("No store is selected.");
    await fulfillOrder(client, order.id, tracking);
    toast.success(`Order #${order.number} marked as fulfilled`);
    setSelected(null);
    reload();
  };

  const handleRefund = async (order: OrderView, amount: number, reason: string) => {
    if (mode === "demo") {
      const refundedTotal = Math.round((order.refundedTotal + amount) * 100) / 100;
      const fullyRefunded = refundedTotal >= order.total - 0.001;
      const updated: OrderView = {
        ...order,
        paymentStatus: fullyRefunded ? "refunded" : "partially_refunded",
        refundedTotal,
        canRefund: !fullyRefunded,
        history: [
          ...order.history,
          { label: "Refund issued", at: new Date().toISOString(), tone: "muted" },
        ],
      };
      setPatched((current) => ({ ...current, [order.id]: updated }));
      setSelected(updated);
      toast.success(`${money(amount, order.currency)} refunded`);
      return;
    }
    if (!client) throw new Error("No store is selected.");
    await refundOrder(client, order, amount, reason);
    toast.success(`${money(amount, order.currency)} refunded`);
    setSelected(null);
    reload();
  };

  const exportOrders = () => {
    downloadCsv(
      `fastcart-orders-${status}.csv`,
      ["Order", "Customer", "Email", "Placed", "Status", "Payment", "Fulfillment", "Total"],
      rows.map((order) => [
        order.number,
        order.customer,
        order.email,
        order.placedAt,
        order.status,
        order.paymentStatus,
        order.fulfillmentStatus,
        order.total.toFixed(2),
      ]),
    );
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Orders"
        description={`${formatCount(total)} orders · ${formatCount(data.counts.pending ?? 0)} awaiting payment · ${formatCount(data.counts.processing ?? 0)} processing`}
        actions={
          <>
            <Button variant="outline" onClick={exportOrders} disabled={rows.length === 0}>
              <Download className="size-4" aria-hidden="true" />
              Export
            </Button>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              aria-label="Refresh orders"
              title="Refresh orders"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} />
            </Button>
          </>
        }
      />

      <div className="flex flex-col gap-3 xl:flex-row xl:items-center">
        <div className="relative w-full xl:w-[240px]">
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
            placeholder="Search orders or customers"
            className="pr-9 pl-9"
            aria-label="Search orders"
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
          {STATUS_TABS.map((tab) => (
            <button
              key={tab.value}
              type="button"
              onClick={() => {
                setStatus(tab.value);
                setPage(1);
              }}
              aria-pressed={status === tab.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center gap-1.5 rounded-full px-2.5 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                status === tab.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {tab.label}
              <span className="font-mono text-[10px] text-faint tnum">
                {formatCount(data.counts[tab.value] ?? 0)}
              </span>
            </button>
          ))}
        </div>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" className="shrink-0 justify-between gap-2">
              <CalendarDays className="size-4 text-muted-foreground" aria-hidden="true" />
              {RANGE_OPTIONS.find((option) => option.value === range)?.label ?? "All time"}
              <ChevronDown className="size-3.5 text-faint" aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-[180px]">
            <DropdownMenuLabel>Date range</DropdownMenuLabel>
            {RANGE_OPTIONS.map((option) => (
              <DropdownMenuItem
                key={option.value}
                onSelect={() => {
                  setRange(option.value);
                  setPage(1);
                }}
              >
                {option.label}
                {range === option.value ? (
                  <Check className="ml-auto size-3.5 text-brand" aria-hidden="true" />
                ) : null}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">
              {status === "all" ? "All orders" : `${ORDER_STATUS_LABELS[status]} orders`}
            </p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              {mode === "demo"
                ? `${rows.length} shown from the demo sample`
                : `${rows.length} most recent of ${formatCount(total)}`}
            </p>
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" className="text-muted-foreground">
                {SORT_OPTIONS.find((option) => option.value === sort)?.label}
                <ChevronDown className="size-3.5" aria-hidden="true" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[170px]">
              {SORT_OPTIONS.map((option) => (
                <DropdownMenuItem
                  key={option.value}
                  onSelect={() => {
                    setSort(option.value);
                    setPage(1);
                  }}
                >
                  {option.label}
                  {sort === option.value ? (
                    <Check className="ml-auto size-3.5 text-brand" aria-hidden="true" />
                  ) : null}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="w-[104px] pl-6">Order</TableHead>
              <TableHead>Customer</TableHead>
              <TableHead className="hidden w-[96px] md:table-cell">Date</TableHead>
              <TableHead className="w-[122px]">Status</TableHead>
              <TableHead className="hidden w-[110px] lg:table-cell">Payment</TableHead>
              <TableHead className="hidden w-[110px] lg:table-cell">Fulfillment</TableHead>
              <TableHead className="w-[92px] text-right">Total</TableHead>
              <TableHead className="w-9 pr-4" aria-label="Open order" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((order) => (
              <TableRow
                key={order.id}
                tabIndex={0}
                onClick={() => setSelected(order)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    setSelected(order);
                  }
                }}
                className="group h-[58px] cursor-pointer outline-none hover:bg-accent/50 focus-visible:bg-accent/60"
              >
                <TableCell className="h-[58px] pl-6">
                  <div className="relative flex h-full items-center pl-3">
                    <StatusRail tone={orderRailTone[order.status]} />
                    <span className="font-mono text-[12px] text-ink-soft">#{order.number}</span>
                  </div>
                </TableCell>
                <TableCell>
                  <div className="flex items-center gap-2.5">
                    <Avatar className="size-6">
                      <AvatarFallback className="text-[9px]">
                        {initials(order.customer)}
                      </AvatarFallback>
                    </Avatar>
                    <div className="min-w-0">
                      <p className="truncate text-[12.5px] leading-tight text-foreground">
                        {order.customer}
                      </p>
                      <p className="mt-0.5 hidden truncate text-[10.5px] text-faint sm:block">
                        {order.email}
                      </p>
                    </div>
                  </div>
                </TableCell>
                <TableCell className="hidden md:table-cell">
                  <p className="text-[12px] text-ink-soft">{monthDay(order.placedAt)}</p>
                  <p className="mt-0.5 font-mono text-[10px] text-faint">
                    {timeOfDay(order.placedAt)}
                  </p>
                </TableCell>
                <TableCell>
                  <OrderPill status={order.status} />
                </TableCell>
                <TableCell className="hidden lg:table-cell">
                  <PaymentPill status={order.paymentStatus} />
                </TableCell>
                <TableCell className="hidden lg:table-cell">
                  <FulfillmentPill status={order.fulfillmentStatus} />
                </TableCell>
                <TableCell className="text-right text-[12.5px] font-medium text-foreground tnum">
                  {money(order.total, order.currency)}
                </TableCell>
                <TableCell className="pr-4">
                  <ChevronRight className="size-4 text-faint transition-colors group-hover:text-muted-foreground" />
                </TableCell>
              </TableRow>
            ))}
            {rows.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={8} className="py-14 text-center">
                  <p className="text-[13px] font-medium text-foreground">No orders found</p>
                  <p className="mt-1 text-[12px] text-muted-foreground">
                    {query.length > 0
                      ? "Try a different customer, email or order number."
                      : "Orders will appear here as customers check out."}
                  </p>
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>

        <div className="flex flex-col gap-3 border-t border-border px-6 py-3.5 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[11.5px] text-muted-foreground">
            {rows.length === 0
              ? "No orders to show"
              : mode === "demo"
                ? `Showing ${start}–${end} of ${data.loaded ?? rows.length} sample orders`
                : `Showing ${start}–${end} of ${formatCount(total)} orders`}
          </p>
          <Pagination page={page} pages={pages} onChange={setPage} />
        </div>
      </Card>

      <OrderDrawer
        order={selectedOrder}
        onClose={() => setSelected(null)}
        onFulfill={handleFulfill}
        onRefund={handleRefund}
      />
    </div>
  );
}
