import { useCallback, useState } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowDownRight, ArrowUpRight, Download, Plus } from "lucide-react";

import { RevenueChart, StatusBreakdown } from "@/components/charts";
import { MetricCard } from "@/components/metric-card";
import { PageHeader } from "@/components/page-header";
import { ProgressBar } from "@/components/progress-bar";
import { ErrorState, PageSkeleton } from "@/components/states";
import { StatusRail, OrderPill, orderRailTone } from "@/components/status-pill";
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
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { useAuth } from "@/lib/auth";
import { demoOverview } from "@/lib/demo";
import {
  dayMonth,
  dayMonthYear,
  downloadCsv,
  initials,
  money,
  moneyWhole,
  shortDate,
} from "@/lib/format";
import { requestCommand } from "@/lib/commands";
import { loadOverview } from "@/lib/loaders";
import type { AttentionItemView, RevenueRange } from "@/lib/models";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const RANGES: Array<{ value: RevenueRange; label: string }> = [
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
  { value: "90d", label: "90d" },
];

function Delta({ delta }: { delta: number }) {
  const positive = delta >= 0;
  return (
    <span
      className={cn(
        "inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-[11px] font-medium",
        positive ? "bg-brand-soft text-brand-ink" : "bg-destructive/10 text-destructive",
      )}
    >
      {positive ? (
        <ArrowUpRight className="size-3" aria-hidden="true" />
      ) : (
        <ArrowDownRight className="size-3" aria-hidden="true" />
      )}
      {positive ? "+" : ""}
      {delta.toFixed(1)}%
    </span>
  );
}

function updatedLabel(iso: string): string {
  const minutes = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
  if (minutes <= 0) return "just now";
  if (minutes === 1) return "1 min ago";
  return `${minutes} min ago`;
}

const attentionTone: Record<AttentionItemView["tone"], string> = {
  warning: "text-warning",
  destructive: "text-destructive",
  info: "text-info",
};

export function OverviewPage() {
  const { mode, client, store, dataKey } = useAuth();
  const navigate = useNavigate();
  const [range, setRange] = useState<RevenueRange>("30d");

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") return Promise.resolve(demoOverview());
      if (!client || !store) return Promise.reject(new Error("No store is selected."));
      return loadOverview(client, store, signal);
    },
    [mode, client, store],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);

  if (!data && loading) return <PageSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const points = data.revenueRanges[range];
  const total = points.reduce((sum, point) => sum + point.value, 0);
  const statusTotal = data.statusSlices.reduce((sum, slice) => sum + slice.value, 0);
  const fulfilledShare =
    statusTotal > 0
      ? Math.round(
          (((data.statusSlices.find((slice) => slice.label === "Completed")?.value ?? 0) +
            (data.statusSlices.find((slice) => slice.label === "Fulfilled")?.value ?? 0)) /
            statusTotal) *
            100,
        )
      : 0;

  const end = new Date();
  const start = new Date(end);
  start.setDate(end.getDate() - 29);
  const rangeDays = range === "7d" ? 7 : range === "30d" ? 30 : 90;
  const currency = store?.currency_code ?? "USD";

  const exportRevenue = () => {
    downloadCsv(
      `fastcart-revenue-${range}.csv`,
      ["Date", "Net revenue"],
      points.map((point) => [point.date, point.value.toFixed(2)]),
    );
  };

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Overview"
        description={`Revenue, orders and stock across ${data.storeName} · ${dayMonth(start.toISOString())} – ${dayMonthYear(end.toISOString())}`}
        actions={
          <>
            <Button variant="outline" onClick={exportRevenue}>
              <Download className="size-4" aria-hidden="true" />
              Export
            </Button>
            <Button
              onClick={() => {
                requestCommand("new-product");
                void navigate({ to: "/products" });
              }}
            >
              <Plus className="size-4" aria-hidden="true" />
              Add product
            </Button>
          </>
        }
      />

      <section aria-label="Key metrics" className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {data.metrics.map((metric) => (
          <MetricCard
            key={metric.key}
            metric={metric}
            display={metric.display ?? false}
            currency={currency}
          />
        ))}
      </section>

      <section className="grid gap-4 lg:grid-cols-3">
        <Card className="p-6 lg:col-span-2">
          <div className="flex items-start justify-between gap-4">
            <p className="text-[13.5px] font-medium text-foreground">Revenue</p>
            <div className="flex h-8 items-center gap-0.5 rounded-full border border-border bg-secondary p-0.5">
              {RANGES.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  onClick={() => setRange(option.value)}
                  aria-pressed={range === option.value}
                  className={cn(
                    "h-full cursor-pointer rounded-full px-3 text-[11.5px] font-medium transition-colors duration-150",
                    range === option.value
                      ? "bg-card text-foreground shadow-xs"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>

          <div className="mt-5 flex flex-wrap items-baseline gap-x-3 gap-y-2">
            <p className="font-display text-[40px] leading-none tracking-[-0.01em] text-foreground tnum">
              {moneyWhole(total, currency)}
            </p>
            <Delta delta={data.revenueDelta} />
            <span className="text-[11.5px] text-faint">{data.revenueComparison}</span>
          </div>

          <div className="mt-6">
            {total > 0 ? (
              <RevenueChart key={range} points={points} height={176} currency={currency} />
            ) : (
              <div className="flex h-[176px] items-center justify-center rounded-lg border border-dashed border-border text-[12px] text-muted-foreground">
                No revenue in this period
              </div>
            )}
          </div>

          <div className="mt-3 flex items-center justify-between gap-4 text-[11px] text-faint">
            <span>
              {rangeDays} days ending {shortDate(end.toISOString())}
            </span>
            <span>Updated {updatedLabel(data.lastSynced)}</span>
          </div>
        </Card>

        <Card className="flex flex-col p-6">
          <div className="flex items-center justify-between gap-4">
            <p className="text-[13.5px] font-medium text-foreground">Order status</p>
            <Link
              to="/orders"
              className="inline-flex items-center gap-1 text-[11.5px] text-muted-foreground hover:text-foreground"
            >
              View orders
              <ArrowUpRight className="size-3" aria-hidden="true" />
            </Link>
          </div>

          <div className="mt-7">
            <StatusBreakdown slices={data.statusSlices} total={Math.max(statusTotal, 1)} />
          </div>

          <div className="mt-auto border-t border-border pt-4">
            <p className="flex items-center gap-1.5 text-[11px] text-faint">
              <span className="size-1.5 rounded-full bg-brand" aria-hidden="true" />
              {fulfilledShare}% fulfilled or completed
            </p>
          </div>
        </Card>
      </section>

      <section className="grid gap-4 lg:grid-cols-3">
        <Card className="overflow-hidden lg:col-span-2">
          <div className="flex items-center justify-between px-6 pt-5 pb-3">
            <p className="text-[13.5px] font-medium text-foreground">Recent orders</p>
            <Link
              to="/orders"
              className="inline-flex items-center gap-1 text-[11.5px] text-muted-foreground hover:text-foreground"
            >
              View all
              <ArrowUpRight className="size-3" aria-hidden="true" />
            </Link>
          </div>

          <Table>
            <TableHeader>
              <TableRow className="border-border/80 hover:bg-transparent">
                <TableHead className="w-[110px] pl-6">Order</TableHead>
                <TableHead>Customer</TableHead>
                <TableHead className="w-[132px]">Status</TableHead>
                <TableHead className="w-[88px] pr-6 text-right">Total</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.recentOrders.map((order) => (
                <TableRow
                  key={order.id}
                  className="cursor-pointer hover:bg-accent/60"
                  onClick={() => void navigate({ to: "/orders" })}
                >
                  <TableCell className="h-[56px] pl-6">
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
                      <span className="text-[12.5px] text-foreground">{order.customer}</span>
                    </div>
                  </TableCell>
                  <TableCell>
                    <OrderPill status={order.status} />
                  </TableCell>
                  <TableCell className="pr-6 text-right text-[12.5px] font-medium text-foreground tnum">
                    {money(order.total, order.currency)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>

        <Card className="flex flex-col p-6">
          <div className="flex items-center justify-between">
            <p className="text-[13.5px] font-medium text-foreground">Needs attention</p>
            <span className="rounded-full border border-border bg-secondary px-2 py-0.5 text-[10.5px] text-muted-foreground">
              {data.attention.length} {data.attention.length === 1 ? "item" : "items"}
            </span>
          </div>

          <ul className="mt-4 flex-1 divide-y divide-border/70">
            {data.attention.map((item) => (
              <li key={item.title} className="py-3.5 first:pt-1">
                <div className="flex items-baseline justify-between gap-3">
                  <p className="text-[12.5px] font-medium text-foreground">{item.title}</p>
                  <span className={cn("font-mono text-[11px] tnum", attentionTone[item.tone])}>
                    {item.value}
                  </span>
                </div>
                <p className="mt-1 text-[11px] text-muted-foreground">{item.detail}</p>
                <ProgressBar className="mt-2.5 h-1" value={item.progress * 100} tone={item.tone} />
              </li>
            ))}
            {data.attention.length === 0 ? (
              <li className="py-8 text-center text-[12px] text-muted-foreground">
                Nothing needs attention.
              </li>
            ) : null}
          </ul>

          <Link
            to="/inventory"
            className="mt-2 inline-flex items-center gap-1 border-t border-border pt-4 text-[11.5px] text-muted-foreground hover:text-foreground"
          >
            Review inventory
            <ArrowUpRight className="size-3" aria-hidden="true" />
          </Link>
        </Card>
      </section>
    </div>
  );
}
