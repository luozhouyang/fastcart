import type { CSSProperties } from "react";

/** Price shape resolved by the storefront data API. */
export interface ProductGridPrice {
  amount?: string;
  amount_minor?: number;
  currency?: string;
}

/** One product card resolved from a `product_list` binding. */
export interface ProductGridItem {
  id: string;
  title: string;
  handle?: string;
  thumbnail?: string | null;
  price?: ProductGridPrice | null;
  available?: boolean;
}

/** Props for the default theme product grid block. */
export interface ProductGridProps {
  id?: string;
  /** Resolved `product_list` data merged into the block props by the host. */
  products?: ProductGridItem[] | null;
  title?: string;
  columns?: number | string;
  emptyText?: string;
  className?: string;
}

/** Renders a responsive grid of product cards. */
export function ProductGrid({ products, title, columns, emptyText, className }: ProductGridProps) {
  const items = Array.isArray(products) ? products : [];
  const parsed = typeof columns === "number" ? columns : Number(columns ?? 3);
  const safeColumns = Number.isFinite(parsed) ? Math.min(6, Math.max(1, Math.trunc(parsed))) : 3;
  const style: CSSProperties = {
    display: "grid",
    gridTemplateColumns: `repeat(${safeColumns}, minmax(0, 1fr))`,
    gap: "1.5rem",
  };

  return (
    <section className={className} data-columns={safeColumns}>
      {title ? <h2>{title}</h2> : null}
      {items.length === 0 ? (
        <p>{emptyText ?? "No products available yet."}</p>
      ) : (
        <div style={style}>
          {items.map((product) => (
            <ProductCard key={product.id} product={product} />
          ))}
        </div>
      )}
    </section>
  );
}

function ProductCard({ product }: { product: ProductGridItem }) {
  const href = product.handle ? `/products/${encodeURIComponent(product.handle)}` : undefined;
  const price = formatPrice(product.price);
  return (
    <article>
      {product.thumbnail ? (
        <a href={href ?? "#"}>
          <img
            src={product.thumbnail}
            alt={product.title}
            loading="lazy"
            style={{ width: "100%", height: "auto", aspectRatio: "1 / 1", objectFit: "cover" }}
          />
        </a>
      ) : null}
      <h3>{href ? <a href={href}>{product.title}</a> : product.title}</h3>
      {price ? <p>{price}</p> : null}
      {product.available === false ? <p>Sold out</p> : null}
    </article>
  );
}

function formatPrice(price: ProductGridPrice | null | undefined): string | null {
  if (!price) {
    return null;
  }
  if (typeof price.amount === "string" && price.amount.length > 0) {
    return price.currency ? `${price.amount} ${price.currency}` : price.amount;
  }
  if (typeof price.amount_minor === "number") {
    const amount = (price.amount_minor / 100).toFixed(2);
    return price.currency ? `${amount} ${price.currency}` : amount;
  }
  return null;
}
