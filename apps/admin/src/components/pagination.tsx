import { ChevronLeft, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

function pageWindow(page: number, pages: number): Array<number | "gap"> {
  if (pages <= 5) return Array.from({ length: pages }, (_, index) => index + 1);
  const values = new Set<number>([1, pages, page - 1, page, page + 1]);
  const ordered = [...values].filter((value) => value >= 1 && value <= pages).sort((a, b) => a - b);
  const result: Array<number | "gap"> = [];
  ordered.forEach((value, index) => {
    const previous = ordered[index - 1];
    if (previous !== undefined && value - previous > 1) result.push("gap");
    result.push(value);
  });
  return result;
}

export function Pagination({
  page,
  pages,
  onChange,
  className,
}: {
  page: number;
  pages: number;
  onChange: (page: number) => void;
  className?: string;
}) {
  if (pages <= 1) return null;
  return (
    <nav className={cn("flex items-center gap-1", className)} aria-label="Pagination">
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => onChange(Math.max(1, page - 1))}
        disabled={page <= 1}
        aria-label="Previous page"
      >
        <ChevronLeft className="size-3.5" />
      </Button>
      {pageWindow(page, pages).map((entry, index) =>
        entry === "gap" ? (
          <span key={`gap-${index}`} className="px-1 font-mono text-[11px] text-faint">
            …
          </span>
        ) : (
          <button
            key={entry}
            type="button"
            onClick={() => onChange(entry)}
            aria-current={entry === page ? "page" : undefined}
            className={cn(
              "h-7 min-w-7 cursor-pointer rounded-md px-1.5 font-mono text-[11px] transition-colors",
              entry === page
                ? "bg-primary text-primary-foreground"
                : "text-muted-foreground hover:bg-accent hover:text-foreground",
            )}
          >
            {entry}
          </button>
        ),
      )}
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={() => onChange(Math.min(pages, page + 1))}
        disabled={page >= pages}
        aria-label="Next page"
      >
        <ChevronRight className="size-3.5" />
      </Button>
    </nav>
  );
}
