import { useId, useState } from "react";

import { MAX_ADD_QUANTITY, useOptionalCart } from "../cart";
import { useThemeRuntime } from "../context";
import type { ProductGridPrice } from "./product-grid";

/** Props accepted by the default theme product detail block. */
export interface ProductDetailProps {
  /** Text shown when the route did not resolve a product. */
  emptyText?: string;
  /** Optional class names for the wrapping element. */
  className?: string;
}

interface RouteVariant {
  id?: string;
  title?: string;
  sku?: string | null;
  price?: ProductGridPrice | null;
  available?: boolean;
}

interface RouteProduct {
  id?: string;
  title?: string;
  handle?: string;
  thumbnail?: string | null;
  price?: ProductGridPrice | null;
  available?: boolean;
  variants?: RouteVariant[];
}

interface RouteImage {
  url?: string;
  alt?: string | null;
}

interface AddStatus {
  kind: "success" | "error";
  message: string;
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
  const description = typeof resource.description === "string" ? resource.description : null;
  const subtitle = typeof resource.subtitle === "string" ? resource.subtitle : null;

  return (
    <ProductDetailView
      key={product.id ?? route.handle}
      product={product}
      images={images}
      imageUrls={imageUrls}
      description={description}
      subtitle={subtitle}
      routeHandle={route.handle}
      className={className}
    />
  );
}

function ProductDetailView({
  product,
  images,
  imageUrls,
  description,
  subtitle,
  routeHandle,
  className,
}: {
  product: RouteProduct;
  images: RouteImage[];
  imageUrls: string[];
  description: string | null;
  subtitle: string | null;
  routeHandle: string;
  className?: string | undefined;
}) {
  const cart = useOptionalCart();
  const [selectedVariantId, setSelectedVariantId] = useState<string | null>(null);
  const [quantity, setQuantity] = useState(1);
  const [adding, setAdding] = useState(false);
  const [status, setStatus] = useState<AddStatus | null>(null);
  const variantGroupName = useId();

  const title = product.title ?? "Product";
  const variants = Array.isArray(product.variants) ? product.variants : [];
  const selectedVariant =
    variants.find((variant) => variant.id !== undefined && variant.id === selectedVariantId) ??
    variants.find((variant) => variant.id !== undefined && variant.available !== false) ??
    variants[0] ??
    null;
  const displayPrice = formatPrice(selectedVariant?.price ?? product.price);
  const hero = imageUrls[0] ?? product.thumbnail ?? null;
  const busy = adding || Boolean(cart?.updating);
  const canPurchase =
    Boolean(cart) &&
    typeof selectedVariant?.id === "string" &&
    selectedVariant.id.length > 0 &&
    selectedVariant.available !== false &&
    product.available !== false;

  async function handleAddToCart() {
    if (!cart || !canPurchase || busy || typeof selectedVariant?.id !== "string") {
      return;
    }
    setAdding(true);
    setStatus(null);
    try {
      await cart.addItem(selectedVariant.id, quantity);
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

  const buttonLabel = busy
    ? "Adding…"
    : canPurchase
      ? "Add to cart"
      : variants.length === 0
        ? "Unavailable"
        : "Sold out";
  const statusId = `${variantGroupName}-status`;

  return (
    <article className={className} data-product-id={product.id} data-route-handle={routeHandle}>
      <div className="fc-product-detail">
        {hero ? (
          <img
            src={hero}
            alt={images.find((image) => image.url === hero)?.alt ?? title}
            loading="eager"
            style={{ width: "100%", height: "auto", aspectRatio: "1 / 1", objectFit: "cover" }}
          />
        ) : null}
        <div>
          <h1>{title}</h1>
          {subtitle ? <p>{subtitle}</p> : null}
          {displayPrice ? <p data-product-price>{displayPrice}</p> : null}
          {product.available === false ? <p>Sold out</p> : null}
          {description ? <div style={{ whiteSpace: "pre-line" }}>{description}</div> : null}

          {cart ? (
            <div className="fc-product-purchase">
              {variants.length > 1 ? (
                <fieldset className="fc-variant-picker">
                  <legend>Choose a variant</legend>
                  {variants.map((variant, index) => {
                    const variantId = variant.id;
                    const disabled =
                      typeof variantId !== "string" ||
                      variantId.length === 0 ||
                      variant.available === false ||
                      busy;
                    const price = formatPrice(variant.price);
                    return (
                      <label key={variantId ?? `${variant.title ?? "variant"}-${index}`}>
                        <input
                          type="radio"
                          name={`fc-variant-${variantGroupName}`}
                          value={variantId ?? ""}
                          checked={Boolean(variantId) && variantId === selectedVariant?.id}
                          disabled={disabled}
                          onChange={() => {
                            if (variantId) {
                              setSelectedVariantId(variantId);
                              setStatus(null);
                            }
                          }}
                        />
                        <span>
                          {variant.title ?? variant.sku ?? "Variant"}
                          {price ? ` — ${price}` : ""}
                          {variant.available === false ? " (sold out)" : ""}
                        </span>
                      </label>
                    );
                  })}
                </fieldset>
              ) : null}

              <div className="fc-product-buy-row">
                <label className="fc-quantity-field" htmlFor={`${variantGroupName}-quantity`}>
                  <span>Quantity</span>
                  <input
                    id={`${variantGroupName}-quantity`}
                    type="number"
                    min={1}
                    max={MAX_ADD_QUANTITY}
                    step={1}
                    value={quantity}
                    disabled={!canPurchase || busy}
                    onChange={(event) => {
                      const parsed = Number.parseInt(event.target.value, 10);
                      setQuantity(
                        Number.isNaN(parsed) ? 1 : Math.min(MAX_ADD_QUANTITY, Math.max(1, parsed)),
                      );
                    }}
                  />
                </label>
                <button
                  type="button"
                  className="fc-button fc-button--primary"
                  onClick={handleAddToCart}
                  disabled={!canPurchase || busy}
                  aria-describedby={status ? statusId : undefined}
                >
                  {buttonLabel}
                </button>
              </div>

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
                alt={image.alt ?? title}
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
