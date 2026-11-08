import { useId, useState, type CSSProperties } from "react";

import { useOptionalCart } from "../cart";

/** Price shape resolved by the storefront data API. */
export interface ProductGridPrice {
  amount?: string;
  amount_minor?: number;
  currency?: string;
}

/** One variant inside a resolved product card. */
export interface ProductGridVariant {
  id?: string;
  title?: string;
  sku?: string | null;
  price?: ProductGridPrice | null;
  available?: boolean;
}

/** One product card resolved from a `product_list` binding. */
export interface ProductGridItem {
  id: string;
  title: string;
  handle?: string;
  thumbnail?: string | null;
  price?: ProductGridPrice | null;
  available?: boolean;
  variants?: ProductGridVariant[] | null;
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
  const variants = Array.isArray(product.variants) ? product.variants : [];
  const singleVariant = variants.length === 1 ? variants[0] : undefined;
  const variantId =
    typeof singleVariant?.id === "string" && singleVariant.id.length > 0 ? singleVariant.id : null;
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
      {variantId ? (
        <CardAddButton
          variantId={variantId}
          productTitle={product.title}
          available={product.available !== false && singleVariant?.available !== false}
        />
      ) : null}
    </article>
  );
}

/**
 * Adds a single-variant product from its card.
 *
 * Products with no variant or several variants deliberately omit this control:
 * the card does not know which variant the customer would choose, so sending a
 * guessed id would add the wrong item.
 */
function CardAddButton({
  variantId,
  productTitle,
  available,
}: {
  variantId: string;
  productTitle: string;
  available: boolean;
}) {
  const cart = useOptionalCart();
  const [adding, setAdding] = useState(false);
  const [status, setStatus] = useState<{ kind: "success" | "error"; message: string } | null>(null);
  const statusId = useId();

  if (!cart) {
    return null;
  }

  const busy = adding || cart.updating;

  async function handleAdd() {
    if (!cart || busy || !available) {
      return;
    }
    setAdding(true);
    setStatus(null);
    try {
      await cart.addItem(variantId, 1);
      setStatus({ kind: "success", message: "Added to cart." });
    } catch (cause) {
      setStatus({
        kind: "error",
        message:
          cause instanceof Error && cause.message.length > 0
            ? cause.message
            : "We couldn't add this item to your cart.",
      });
    } finally {
      setAdding(false);
    }
  }

  return (
    <div className="fc-product-card-action">
      <button
        type="button"
        className="fc-button fc-button--secondary fc-button--small"
        onClick={handleAdd}
        disabled={busy || !available}
        aria-label={available ? `Add ${productTitle} to cart` : `${productTitle} is sold out`}
        aria-describedby={status ? statusId : undefined}
      >
        {busy ? "Adding…" : available ? "Add to cart" : "Sold out"}
      </button>
      {status ? (
        <p
          id={statusId}
          className={`fc-status fc-status--${status.kind}`}
          role={status.kind === "error" ? "alert" : "status"}
        >
          {status.message}
        </p>
      ) : null}
    </div>
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
