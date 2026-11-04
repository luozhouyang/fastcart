import { useEffect, useId, useState } from "react";
import type { PointerEvent } from "react";

import { moneyWhole } from "@/lib/format";
import type { RevenuePointView, StatusSliceView } from "@/lib/models";
import { cn } from "@/lib/utils";

interface Point {
  x: number;
  y: number;
}

function smoothPath(points: Point[]): string {
  if (points.length === 0) return "";
  const first = points[0];
  if (!first) return "";
  if (points.length === 1) return `M ${first.x} ${first.y}`;

  let path = `M ${first.x} ${first.y}`;
  for (let index = 0; index < points.length - 1; index += 1) {
    const current = points[index];
    const next = points[index + 1];
    if (!current || !next) continue;
    const previous = points[index - 1] ?? current;
    const after = points[index + 2] ?? next;
    const cp1x = current.x + (next.x - previous.x) / 6;
    const cp1y = current.y + (next.y - previous.y) / 6;
    const cp2x = next.x - (after.x - current.x) / 6;
    const cp2y = next.y - (after.y - current.y) / 6;
    path += ` C ${cp1x} ${cp1y}, ${cp2x} ${cp2y}, ${next.x} ${next.y}`;
  }
  return path;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function RevenueChart({
  points,
  height = 252,
  currency = "USD",
}: {
  points: RevenuePointView[];
  height?: number;
  currency?: string;
}) {
  const gradientId = `revenue-${useId().replace(/:/g, "")}`;
  const clipId = `reveal-${useId().replace(/:/g, "")}`;
  const [active, setActive] = useState<number | null>(null);

  const values = points.map((point) => point.value);
  const max = Math.ceil((Math.max(...values, 1) * 1.12) / 500) * 500;
  const top = 6;
  const bottom = 6;

  const coordinates: Point[] = points.map((point, index) => ({
    x: (index / Math.max(points.length - 1, 1)) * 100,
    y: bottom + (1 - point.value / max) * (100 - top - bottom),
  }));

  const line = smoothPath(coordinates);
  const first = coordinates[0];
  const last = coordinates[coordinates.length - 1];
  const area = `${line} L ${last?.x ?? 100} 100 L ${first?.x ?? 0} 100 Z`;

  const xLabels = [0, 0.25, 0.5, 0.75, 1].map(
    (fraction) => points[Math.round(fraction * (points.length - 1))]?.date ?? "",
  );

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const ratio = clamp((event.clientX - rect.left) / rect.width, 0, 1);
    setActive(Math.round(ratio * (points.length - 1)));
  };

  const activePoint = active !== null ? points[active] : undefined;
  const activeCoordinate = active !== null ? coordinates[active] : undefined;

  return (
    <div>
      <div
        className="relative w-full touch-none select-none"
        style={{ height }}
        onPointerMove={onPointerMove}
        onPointerLeave={() => setActive(null)}
        role="img"
        aria-label={`Revenue trend over ${points.length} days`}
      >
        <svg
          viewBox="0 0 100 100"
          preserveAspectRatio="none"
          className="absolute inset-0 h-full w-full overflow-visible"
          aria-hidden="true"
        >
          <defs>
            <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--brand)" stopOpacity={0.2} />
              <stop offset="68%" stopColor="var(--brand)" stopOpacity={0.04} />
              <stop offset="100%" stopColor="var(--brand)" stopOpacity={0} />
            </linearGradient>
            <clipPath id={clipId}>
              <rect x={0} y={0} width={100} height={100} className="animate-reveal" />
            </clipPath>
          </defs>

          {[0, 25, 50, 75, 100].map((y) => (
            <line
              key={y}
              x1={0}
              x2={100}
              y1={y}
              y2={y}
              stroke="var(--border)"
              strokeWidth={1}
              strokeDasharray={y === 100 ? undefined : "2 6"}
              vectorEffect="non-scaling-stroke"
              opacity={y === 100 ? 0.9 : 0.6}
            />
          ))}

          <g clipPath={`url(#${clipId})`}>
            <path d={area} fill={`url(#${gradientId})`} className="animate-fade" />
            <path
              d={line}
              fill="none"
              stroke="var(--brand)"
              strokeWidth={2}
              strokeLinecap="round"
              strokeLinejoin="round"
              vectorEffect="non-scaling-stroke"
            />
          </g>
        </svg>

        {activeCoordinate && activePoint ? (
          <>
            <div
              className="pointer-events-none absolute top-0 bottom-0 w-px bg-foreground/15"
              style={{ left: `${activeCoordinate.x}%` }}
            />
            <div
              className="pointer-events-none absolute z-10 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-card bg-brand shadow-[0_0_0_1px_var(--brand)]"
              style={{ left: `${activeCoordinate.x}%`, top: `${activeCoordinate.y}%` }}
            />
            <div
              className={cn(
                "pointer-events-none absolute z-20 -translate-y-[calc(100%+14px)] rounded-lg border border-border bg-popover px-3 py-2 shadow-lift",
                active === 0 && "translate-x-0",
                active === points.length - 1 && "-translate-x-full",
                active !== 0 && active !== points.length - 1 && "-translate-x-1/2",
              )}
              style={{ left: `${activeCoordinate.x}%`, top: `${activeCoordinate.y}%` }}
            >
              <p className="font-mono text-[10px] text-faint">{activePoint.date}</p>
              <p className="mt-0.5 text-[13px] font-medium text-foreground tnum">
                {moneyWhole(activePoint.value, currency)}
              </p>
            </div>
          </>
        ) : null}
      </div>

      <div className="mt-2 flex justify-between font-mono text-[10px] text-faint">
        {xLabels.map((label, index) => (
          <span key={`${label}-${index}`}>{label}</span>
        ))}
      </div>
    </div>
  );
}

export function StatusBreakdown({ slices, total }: { slices: StatusSliceView[]; total: number }) {
  const [mounted, setMounted] = useState(false);
  const [hovered, setHovered] = useState<number | null>(null);

  useEffect(() => {
    const frame = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(frame);
  }, []);

  return (
    <div>
      <div className="flex h-2.5 w-full gap-[3px] overflow-hidden rounded-full" aria-hidden="true">
        {slices.map((slice, index) => (
          <span
            key={slice.label}
            className="h-full rounded-full transition-[width,opacity] duration-700 ease-out"
            style={{
              width: mounted ? `${(slice.value / total) * 100}%` : "0%",
              backgroundColor: slice.color,
              opacity: hovered === null || hovered === index ? 1 : 0.28,
            }}
          />
        ))}
      </div>

      <ul className="mt-5 space-y-0.5">
        {slices.map((slice, index) => (
          <li key={slice.label}>
            <button
              type="button"
              onMouseEnter={() => setHovered(index)}
              onMouseLeave={() => setHovered(null)}
              onFocus={() => setHovered(index)}
              onBlur={() => setHovered(null)}
              className={cn(
                "flex w-full cursor-default items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors duration-150 hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/25 focus-visible:outline-none",
                hovered !== null && hovered !== index && "opacity-55",
              )}
            >
              <span
                className="size-2 shrink-0 rounded-full"
                style={{ backgroundColor: slice.color }}
                aria-hidden="true"
              />
              <span className="flex-1 text-[12.5px] text-ink-soft">{slice.label}</span>
              <span className="text-[12.5px] font-medium text-foreground tnum">
                {slice.value.toLocaleString("en-US")}
              </span>
              <span className="w-10 text-right font-mono text-[10.5px] text-faint tnum">
                {Math.round((slice.value / total) * 100)}%
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
