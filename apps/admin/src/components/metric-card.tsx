import { ArrowDownRight, ArrowUpRight, Minus } from "lucide-react";

import { Card } from "@/components/ui/card";
import { useCountUp } from "@/lib/hooks";
import { money, moneyWhole, percent } from "@/lib/format";
import { cn } from "@/lib/utils";

export interface Metric {
  key: string;
  label: string;
  value: number;
  format: "money" | "moneyWhole" | "number" | "percent";
  delta: number;
  comparison: string;
}

function formatMetric(value: number, format: Metric["format"], currency: string): string {
  if (format === "moneyWhole") return moneyWhole(value, currency);
  if (format === "money") return money(value, currency);
  if (format === "percent") return percent(value);
  return Math.round(value).toLocaleString("en-US");
}

export function MetricCard({
  metric,
  display = false,
  currency = "USD",
}: {
  metric: Metric;
  display?: boolean;
  currency?: string;
}) {
  const animated = useCountUp(metric.value);
  const neutral = metric.delta === 0;
  const positive = metric.delta > 0;

  return (
    <Card className="p-6">
      <p className="text-[12.5px] font-medium text-muted-foreground">{metric.label}</p>
      <p
        className={cn(
          "mt-3 text-foreground tnum",
          display
            ? "font-display text-[42px] leading-none tracking-[-0.01em]"
            : "text-[32px] leading-none font-medium tracking-[-0.02em]",
        )}
      >
        {formatMetric(animated, metric.format, currency)}
      </p>
      <div className="mt-4 flex flex-wrap items-center gap-x-2 gap-y-1.5 text-[11.5px]">
        <span
          className={cn(
            "inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 font-medium",
            neutral
              ? "bg-secondary text-muted-foreground"
              : positive
                ? "bg-brand-soft text-brand-ink"
                : "bg-destructive/10 text-destructive",
          )}
        >
          {neutral ? (
            <Minus className="size-3" aria-hidden="true" />
          ) : positive ? (
            <ArrowUpRight className="size-3" aria-hidden="true" />
          ) : (
            <ArrowDownRight className="size-3" aria-hidden="true" />
          )}
          {neutral ? "No change" : `${positive ? "+" : ""}${metric.delta.toFixed(1)}%`}
        </span>
        <span className="text-faint">{metric.comparison}</span>
      </div>
    </Card>
  );
}

export function StatCard({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <Card className="p-5">
      <p className="text-[12px] text-muted-foreground">{label}</p>
      <p className="mt-2 text-[24px] leading-none font-medium tracking-[-0.02em] text-foreground tnum">
        {value}
      </p>
      {hint ? <p className="mt-2.5 text-[11px] text-faint">{hint}</p> : null}
    </Card>
  );
}
