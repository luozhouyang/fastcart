import { useThemeRuntime } from "../context";
import { ProductGrid, type ProductGridItem } from "./product-grid";

/** Props accepted by the default theme collection detail block. */
export interface CollectionDetailProps {
  /** Optional heading override. */
  title?: string;
  /** Text shown when the route did not resolve a collection. */
  emptyText?: string;
  /** Optional class names for the wrapping element. */
  className?: string;
}

/**
 * Renders the collection resolved for a dynamic `/collections/{handle}` route
 * together with its published products.
 */
export function CollectionDetail({ title, emptyText, className }: CollectionDetailProps) {
  const { route } = useThemeRuntime();
  if (!route || route.kind !== "collection" || !isRecord(route.resource)) {
    return <p>{emptyText ?? "Collection not found."}</p>;
  }
  const resource = route.resource;
  const collection = isRecord(resource.collection) ? resource.collection : null;
  if (!collection || typeof collection.title !== "string") {
    return <p>{emptyText ?? "Collection not found."}</p>;
  }
  const products = Array.isArray(resource.products) ? (resource.products as ProductGridItem[]) : [];
  const description = typeof collection.description === "string" ? collection.description : null;

  return (
    <section
      className={className}
      data-collection-id={collection.id}
      data-route-handle={route.handle}
    >
      <h1>{title ?? collection.title}</h1>
      {description ? <p>{description}</p> : null}
      <ProductGrid products={products} emptyText="No products in this collection yet." />
    </section>
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
