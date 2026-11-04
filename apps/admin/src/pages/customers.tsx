import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Check,
  ChevronDown,
  Copy,
  Download,
  MoreHorizontal,
  ReceiptText,
  Search,
  X,
} from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";

import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
import { CustomerStatusPill } from "@/components/status-pill";
import { ErrorState, TableSkeleton } from "@/components/states";
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
import { demoCustomers } from "@/lib/demo";
import {
  downloadCsv,
  initials,
  money,
  moneyShort,
  moneyWhole,
  percent,
  shortDate,
} from "@/lib/format";
import { loadCustomers } from "@/lib/loaders";
import type { CustomerView, MetricView } from "@/lib/models";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 10;

type SortMode = "recent" | "spend" | "joined";
type FilterValue = "all" | "active" | "new" | "guest";

const FILTERS: Array<{ value: FilterValue; label: string }> = [
  { value: "all", label: "All" },
  { value: "active", label: "Active" },
  { value: "new", label: "New" },
  { value: "guest", label: "Guests" },
];

const SORT_OPTIONS: Array<{ value: SortMode; label: string }> = [
  { value: "recent", label: "Recently active" },
  { value: "spend", label: "Highest spend" },
  { value: "joined", label: "Newest first" },
];

function metricValue(metric: MetricView, currency: string): string {
  if (metric.format === "money") return money(metric.value, currency);
  if (metric.format === "moneyWhole") return moneyWhole(metric.value, currency);
  if (metric.format === "percent") return percent(metric.value);
  return Math.round(metric.value).toLocaleString("en-US");
}

export function CustomersPage() {
  const { mode, client, store, dataKey } = useAuth();
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<FilterValue>("all");
  const [sort, setSort] = useState<SortMode>("recent");
  const [page, setPage] = useState(1);

  useEffect(() => {
    setPage(1);
    setFilter("all");
    setQuery("");
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") return Promise.resolve(demoCustomers());
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadCustomers(client, signal);
    },
    [mode, client],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);

  const customers = data?.customers ?? [];

  const counts = useMemo(
    () => ({
      all: customers.length,
      active: customers.filter((customer) => customer.status === "active").length,
      new: customers.filter((customer) => customer.status === "new").length,
      guest: customers.filter((customer) => customer.status === "guest").length,
    }),
    [customers],
  );

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const list = customers.filter((customer) => {
      if (filter !== "all" && customer.status !== filter) return false;
      if (needle.length === 0) return true;
      return (
        customer.name.toLowerCase().includes(needle) ||
        customer.email.toLowerCase().includes(needle)
      );
    });
    return list.sort((a, b) => {
      if (sort === "spend") return b.spent - a.spent;
      if (sort === "joined") return b.joined.localeCompare(a.joined);
      return b.lastOrder.localeCompare(a.lastOrder);
    });
  }, [customers, filter, query, sort]);

  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, pages);
  const offset = (currentPage - 1) * PAGE_SIZE;
  const rows = filtered.slice(offset, offset + PAGE_SIZE);
  const start = rows.length === 0 ? 0 : offset + 1;
  const end = offset + rows.length;
  const currency = store?.currency_code ?? "USD";
  const returningMetric = data?.metrics.find((metric) => metric.key === "returning");
  const total = data?.total ?? 0;

  const exportCustomers = () => {
    downloadCsv(
      "fastcart-customers.csv",
      ["Name", "Email", "Status", "Orders", "Total spent", "Last order", "Joined"],
      filtered.map((customer) => [
        customer.name,
        customer.email,
        customer.status,
        customer.orders,
        customer.spent.toFixed(2),
        customer.lastOrder,
        customer.joined,
      ]),
    );
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Customers"
        description={`${total.toLocaleString("en-US")} customers · ${percent(returningMetric?.value ?? 0)} returning`}
        actions={
          <Button variant="outline" onClick={exportCustomers} disabled={filtered.length === 0}>
            <Download className="size-4" aria-hidden="true" />
            Export
          </Button>
        }
      />

      <section className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4" aria-label="Customer metrics">
        {data.metrics.map((metric) => (
          <Card key={metric.key} className="p-5">
            <p className="text-[12px] text-muted-foreground">{metric.label}</p>
            <div className="mt-3 flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
              <p className="text-[30px] leading-none font-medium tracking-[-0.02em] text-foreground tnum">
                {metricValue(metric, currency)}
              </p>
              <p className="text-[10.5px] text-faint">{metric.comparison}</p>
            </div>
          </Card>
        ))}
      </section>

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
            placeholder="Search customers by name or email"
            className="pr-9 pl-9"
            aria-label="Search customers"
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
          {FILTERS.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setFilter(option.value);
                setPage(1);
              }}
              aria-pressed={filter === option.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center gap-1.5 rounded-full px-3 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                filter === option.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
              <span className="font-mono text-[10px] text-faint tnum">
                {counts[option.value].toLocaleString("en-US")}
              </span>
            </button>
          ))}
        </div>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" className="shrink-0 justify-between gap-2">
              {SORT_OPTIONS.find((option) => option.value === sort)?.label}
              <ChevronDown className="size-3.5 text-faint" aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-[190px]">
            <DropdownMenuLabel>Sort customers</DropdownMenuLabel>
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

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">
              {filter === "all"
                ? "All customers"
                : `${FILTERS.find((entry) => entry.value === filter)?.label ?? ""} customers`}
            </p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              {mode === "demo"
                ? `${filtered.length} in the demo sample`
                : `${filtered.length.toLocaleString("en-US")} matched of ${total.toLocaleString("en-US")}`}
            </p>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="text-muted-foreground"
            onClick={exportCustomers}
          >
            Export segment
          </Button>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Customer</TableHead>
              <TableHead className="w-[100px]">Status</TableHead>
              <TableHead className="w-[80px] text-right">Orders</TableHead>
              <TableHead className="w-[120px] text-right">Total spent</TableHead>
              <TableHead className="hidden w-[120px] md:table-cell">Last order</TableHead>
              <TableHead className="hidden w-[120px] lg:table-cell">Joined</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((customer) => (
              <CustomerRow
                key={customer.id}
                customer={customer}
                currency={currency}
                onViewOrders={() => void navigate({ to: "/orders" })}
              />
            ))}
            {rows.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={7} className="py-14 text-center">
                  <p className="text-[13px] font-medium text-foreground">No customers found</p>
                  <p className="mt-1 text-[12px] text-muted-foreground">
                    Try a different name, email or status.
                  </p>
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>

        <div className="flex flex-col gap-3 border-t border-border px-6 py-3.5 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[11.5px] text-muted-foreground">
            {filtered.length === 0
              ? "No customers to show"
              : mode === "demo"
                ? `Showing ${start}–${end} of ${filtered.length} sample customers · ${total.toLocaleString("en-US")} total`
                : `Showing ${start}–${end} of ${filtered.length.toLocaleString("en-US")} matched customers`}
          </p>
          <Pagination page={currentPage} pages={pages} onChange={setPage} />
        </div>
      </Card>
    </div>
  );
}

function CustomerRow({
  customer,
  currency,
  onViewOrders,
}: {
  customer: CustomerView;
  currency: string;
  onViewOrders: () => void;
}) {
  const copyEmail = async () => {
    try {
      await navigator.clipboard.writeText(customer.email);
      toast.success("Email copied");
    } catch {
      toast.error("Could not copy the email address.");
    }
  };

  return (
    <TableRow className="group h-[58px] hover:bg-accent/40">
      <TableCell className="pl-6">
        <div className="flex items-center gap-3">
          <Avatar className="size-8">
            <AvatarFallback className="text-[10px]">{initials(customer.name)}</AvatarFallback>
          </Avatar>
          <div className="min-w-0">
            <p className="truncate text-[12.5px] leading-tight font-medium text-foreground">
              {customer.name}
            </p>
            <p className="mt-0.5 truncate text-[10.5px] text-faint">{customer.email}</p>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <CustomerStatusPill status={customer.status} />
      </TableCell>
      <TableCell className="text-right font-mono text-[11.5px] text-ink-soft tnum">
        {customer.orders}
      </TableCell>
      <TableCell className="text-right text-[12px] text-ink-soft tnum">
        {moneyShort(customer.spent, currency)}
      </TableCell>
      <TableCell className="hidden text-[11.5px] text-muted-foreground md:table-cell">
        {shortDate(customer.lastOrder)}
      </TableCell>
      <TableCell className="hidden text-[11.5px] text-muted-foreground lg:table-cell">
        {shortDate(customer.joined)}
      </TableCell>
      <TableCell className="pr-5 text-right">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={`Actions for ${customer.name}`}
              className="text-faint data-[state=open]:bg-accent"
            >
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-[190px]">
            <DropdownMenuLabel>{customer.email}</DropdownMenuLabel>
            <DropdownMenuItem onSelect={onViewOrders}>
              <ReceiptText aria-hidden="true" />
              View orders
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void copyEmail()}>
              <Copy aria-hidden="true" />
              Copy email
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  );
}
