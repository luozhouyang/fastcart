/** Props for the fallback rendered when a page references an unknown block. */
export interface UnknownBlockProps {
  type?: string;
}

/**
 * Renders a harmless placeholder for a block type this theme does not know.
 *
 * The element keeps the block type in a data attribute so previews and
 * debugging tools can identify stale content without leaking a component name
 * into visible copy.
 */
export function UnknownBlock({ type }: UnknownBlockProps) {
  return <div className="fc-block-unknown" data-block-type={type ?? "unknown"} hidden />;
}
