import { cn } from "@/lib/utils";

export function ProductThumb({
  title,
  swatch,
  className,
  initialClassName,
}: {
  title: string;
  swatch: [string, string];
  className?: string;
  initialClassName?: string;
}) {
  const initial = title.trim().charAt(0).toUpperCase() || "P";

  return (
    <div
      aria-hidden="true"
      className={cn("relative flex items-center justify-center overflow-hidden", className)}
      style={{ backgroundImage: `linear-gradient(145deg, ${swatch[0]}, ${swatch[1]})` }}
    >
      <span
        className={cn(
          "font-display text-[44px] leading-none text-white/80 mix-blend-soft-light",
          initialClassName,
        )}
      >
        {initial}
      </span>
      <span className="pointer-events-none absolute inset-0 ring-1 ring-inset ring-black/[0.06] dark:ring-white/[0.08]" />
    </div>
  );
}
