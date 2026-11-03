import { Button } from "@fastcart/ui";
import { Link, Outlet, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";

const rootRoute = createRootRoute({
  component: RootLayout,
});

function RootLayout() {
  return (
    <div className="min-h-screen bg-slate-950 text-slate-100">
      <header className="border-b border-white/10">
        <div className="mx-auto flex max-w-5xl items-center gap-6 px-6 py-4">
          <span className="font-semibold tracking-tight">Fastcart Admin</span>
          <nav className="flex gap-4 text-sm text-slate-400">
            <Link to="/" activeProps={{ className: "text-emerald-400" }}>
              Dashboard
            </Link>
            <Link to="/products" activeProps={{ className: "text-emerald-400" }}>
              Products
            </Link>
          </nav>
        </div>
      </header>
      <main className="mx-auto max-w-5xl px-6 py-8">
        <Outlet />
      </main>
    </div>
  );
}

function DashboardPage() {
  return (
    <section className="space-y-4">
      <h1 className="text-2xl font-semibold">Dashboard</h1>
      <p className="text-sm text-slate-400">
        Phase 0 shell. Store metrics and order summaries arrive with the admin API.
      </p>
      <Button>Create product</Button>
    </section>
  );
}

function ProductsPage() {
  return (
    <section className="space-y-4">
      <h1 className="text-2xl font-semibold">Products</h1>
      <div className="rounded-lg border border-white/10 p-6 text-sm text-slate-400">
        Product management UI is wired to the admin SDK in a later phase.
      </div>
    </section>
  );
}

const dashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: DashboardPage,
});

const productsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/products",
  component: ProductsPage,
});

const routeTree = rootRoute.addChildren([dashboardRoute, productsRoute]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
