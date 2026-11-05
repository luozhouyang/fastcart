import { useThemeRuntime } from "../context";
import type { ProductGridPrice } from "./product-grid";

/** Props accepted by the default theme product detail block. */
export interface ProductDetailProps {
  /** Text shown when the route did not resolve a product. */
  emptyText?: string;
  /** Optional class names for the wrapping element. */
  className?: string;
}

interface RouteProduct {
  id?: string;
  title?: string;
  handle?: string;
  thumbnail?: string | null;
  price?: ProductGridPrice | null;
  available?: boolean;
  variants?: Array<{
    id?: string;
    title?: string;
    sku?: string | null;
    price?: ProductGridPrice | null;
    available?: boolean;
  }>;
}

interface RouteImage {
  url?: string;
  alt?: string | null;
}

/**
 * Renders the product resolved for a dynamic `/products/{handle}` route.
 *
 * The host injects the resource into the reserved `context` data entry; this
 * block never receives product data through page props, so the same template
 * page serves every product.
 */
export function ProductDetail({ emptyText, className }: ProductDetailProps) {
  const { route } = useThemeRuntime();
  if (!route || route.kind !== "product" || !isRecord(route.resource)) {
    return <p>{emptyText ?? "Product not found."}</p>;
  }
  const resource = route.resource;
  const product = isRecord(resource.product) ? (resource.product as RouteProduct) : null;
  if (!product || typeof product.title !== "string") {
    return <p>{emptyText ?? "Product not found."}</p>;
  }

  const images: RouteImage[] = Array.isArray(resource.images)
    ? resource.images.filter(isRecord).map((image) => ({
        ...(typeof image.url === "string" ? { url: image.url } : {}),
        alt: typeof image.alt === "string" ? image.alt : null,
      }))
    : [];
  const imageUrls = images
    .map((image) => image.url)
    .filter((url): url is string => typeof url === "string" && url.length > 0);
  const hero = imageUrls[0] ?? product.thumbnail ?? null;
  const description = typeof resource.description === "string" ? resource.description : null;
  const subtitle = typeof resource.subtitle === "string" ? resource.subtitle : null;
  const variants = Array.isArray(product.variants) ? product.variants : [];

  return (
    <article className={className} data-product-id={product.id} data-route-handle={route.handle}>
      <div className="fc-product-detail">
        {hero ? (
          <img
            src={hero}
            alt={images.find((image) => image.url === hero)?.alt ?? product.title}
            loading="eager"
            style={{ width: "100%", height: "auto", aspectRatio: "1 / 1", objectFit: "cover" }}
          />
        ) : null}
        <div>
          <h1>{product.title}</h1>
          {subtitle ? <p>{subtitle}</p> : null}
          {formatPrice(product.price) ? (
            <p data-product-price>{formatPrice(product.price)}</p>
          ) : null}
          {product.available === false ? <p>Sold out</p> : null}
          {description ? <div style={{ whiteSpace: "pre-line" }}>{description}</div> : null}
          {variants.length > 1 ? (
            <ul data-product-variants>
              {variants.map((variant, index) => (
                <li key={variant.id ?? `${variant.title ?? "variant"}-${index}`}>
                  {variant.title ?? variant.sku ?? "Variant"}
                  {formatPrice(variant.price) ? ` — ${formatPrice(variant.price)}` : ""}
                  {variant.available === false ? " (sold out)" : ""}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </div>
      {imageUrls.length > 1 ? (
        <div className="fc-product-gallery">
          {images.map((image, index) =>
            image.url ? (
              <img
                key={`${image.url}-${index}`}
                src={image.url}
                alt={image.alt ?? product.title}
                loading="lazy"
              />
            ) : null,
          )}
        </div>
      ) : null}
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
