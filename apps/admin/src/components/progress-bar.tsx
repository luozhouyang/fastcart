import { useEffect, useState } from "react";

import { cn } from "@/lib/utils";

export function ProgressBar({
  value,
  max = 100,
  tone = "brand",
  className,
}: {
  value: number;
  max?: number;
  tone?: "brand" | "warning" | "destructive" | "info";
  className?: string;
}) {
  const [width, setWidth] = useState(0);
  const target = Math.max(0, Math.min(100, (value / Math.max(max, 1)) * 100));

  useEffect(() => {
    const frame = requestAnimationFrame(() => setWidth(target));
    return () => cancelAnimationFrame(frame);
  }, [target]);

  const tones = {
    brand: "bg-brand",
    warning: "bg-warning",
    destructive: "bg-destructive",
    info: "bg-info",
  } as const;

  return (
    <span className={cn("block h-1.5 w-full overflow-hidden rounded-full bg-secondary", className)}>
      <span
        className={cn(
          "block h-full rounded-full transition-[width] duration-700 ease-out",
          tones[tone],
        )}
        style={{ width: `${width}%` }}
      />
    </span>
  );
}
