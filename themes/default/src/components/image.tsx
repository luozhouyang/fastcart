/** Resolved `asset` data binding shape. */
export interface ImageAsset {
  id?: string;
  url?: string | null;
  alt?: string | null;
  width?: number | null;
  height?: number | null;
  content_type?: string | null;
}

/** Props for the default theme image block. */
export interface ImageProps {
  id?: string;
  /** Resolved asset binding merged into the block props by the host. */
  asset?: ImageAsset | null;
  /** Explicit source, taking precedence over the resolved asset. */
  src?: string;
  alt?: string;
  width?: number;
  height?: number;
  href?: string;
  loading?: "lazy" | "eager";
  className?: string;
}

/** Renders a responsive image, optionally wrapped in a link. */
export function Image({ asset, src, alt, width, height, href, loading, className }: ImageProps) {
  const resolvedSrc = src ?? asset?.url ?? undefined;
  if (!resolvedSrc) {
    return null;
  }
  const image = (
    <img
      src={resolvedSrc}
      alt={alt ?? asset?.alt ?? ""}
      width={width ?? asset?.width ?? undefined}
      height={height ?? asset?.height ?? undefined}
      loading={loading ?? "lazy"}
      className={className}
    />
  );
  return href ? <a href={href}>{image}</a> : image;
}
