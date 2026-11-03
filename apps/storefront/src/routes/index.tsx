import { Link, createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: HomePage,
});

function HomePage() {
  return (
    <section className="space-y-6">
      <p className="text-sm font-medium uppercase tracking-widest text-emerald-600">
        Storefront SSR
      </p>
      <h1 className="max-w-2xl text-4xl font-semibold tracking-tight">
        A TanStack Start storefront served by the Fastcart Rust runtime.
      </h1>
      <p className="max-w-2xl text-slate-600">
        This Phase 0 page verifies routing and SSR. Commerce loaders are connected through the
        internal fetch bridge in Phase 6.
      </p>
      <Link
        to="/products/$handle"
        params={{ handle: "example-product" }}
        className="inline-flex rounded-md bg-slate-900 px-4 py-2 text-sm font-medium text-white"
      >
        View example product
      </Link>
    </section>
  );
}
