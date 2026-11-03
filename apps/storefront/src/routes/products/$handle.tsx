import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/products/$handle")({
  component: ProductPage,
});

function ProductPage() {
  const { handle } = Route.useParams();

  return (
    <article className="space-y-4">
      <p className="text-xs uppercase tracking-widest text-slate-500">Product</p>
      <h1 className="text-3xl font-semibold tracking-tight">{handle}</h1>
      <p className="max-w-2xl text-slate-600">
        Product data will be loaded from the Rust commerce service through the internal fetch
        bridge.
      </p>
    </article>
  );
}
