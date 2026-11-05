/** Props for the default theme heading block. */
export interface HeadingProps {
  id?: string;
  text?: string;
  content?: string;
  /** Heading level; values outside 1-6 are clamped. */
  level?: number | string;
  className?: string;
}

/** Renders an `h1`-`h6` heading. */
export function Heading({ text, content, level, className }: HeadingProps) {
  const value = text ?? content ?? "";
  if (value.length === 0) {
    return null;
  }
  const parsed = typeof level === "number" ? level : Number(level ?? 2);
  const safeLevel = Number.isFinite(parsed) ? Math.min(6, Math.max(1, Math.trunc(parsed))) : 2;
  const Tag = `h${safeLevel}` as "h1" | "h2" | "h3" | "h4" | "h5" | "h6";
  return <Tag className={className}>{value}</Tag>;
}
