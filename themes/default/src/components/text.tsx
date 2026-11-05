import type { CSSProperties } from "react";

/** Props for the default theme text block. */
export interface TextProps {
  id?: string;
  text?: string;
  content?: string;
  className?: string;
}

/** Renders a paragraph of plain text. Newlines are preserved. */
export function Text({ text, content, className }: TextProps) {
  const value = text ?? content ?? "";
  if (value.length === 0) {
    return null;
  }
  const style: CSSProperties = { whiteSpace: "pre-line" };
  return (
    <p className={className} style={style}>
      {value}
    </p>
  );
}
