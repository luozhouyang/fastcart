import React from "react";
import { Link, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";

import { AppShell } from "@/components/app-shell";
import { ThemeProvider, useTheme } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { PermissionGate } from "@/components/permission-gate";
import { PERMISSIONS, type Permission } from "@/lib/permissions";
import { AuthProvider, useAuth } from "@/lib/auth";
import { CustomersPage } from "@/pages/customers";
import { FormEditorPage } from "@/pages/form-editor";
import { FormsPage } from "@/pages/forms";
import { InboxPage } from "@/pages/inbox";
import { InventoryPage } from "@/pages/inventory";
import { LoginPage, NoStorePage, SplashScreen } from "@/pages/login";
import { NotificationsPage } from "@/pages/notifications";
import { OrdersPage } from "@/pages/orders";
import { OverviewPage } from "@/pages/overview";
import { PageEditorPage } from "@/pages/page-editor";
import { PagesPage } from "@/pages/pages";
import { ProductsPage } from "@/pages/products";
import { PromotionsPage } from "@/pages/promotions";
import { RegionsPage } from "@/pages/regions";
import { SettingsPage } from "@/pages/settings";
import { ShippingPage } from "@/pages/shipping";
import { TaxPage } from "@/pages/tax";
import { ThemeDetailPage } from "@/pages/theme-detail";
import { ThemesPage } from "@/pages/themes";

function AppGate() {
  const { status } = useAuth();
  const { theme } = useTheme();

  return (
    <>
      {status === "loading" ? (
        <SplashScreen />
      ) : status === "anonymous" ? (
        <LoginPage />
      ) : status === "no-store" ? (
        <NoStorePage />
      ) : (
        <AppShell />
      )}
      <Toaster theme={theme} />
    </>
  );
}

function RootLayout() {
  return (
    <ThemeProvider>
      <AuthProvider>
        <AppGate />
      </AuthProvider>
    </ThemeProvider>
  );
}

function NotFoundPage() {
  return (
    <div className="flex min-h-[50vh] flex-col items-center justify-center text-center">
      <p className="font-mono text-[10.5px] tracking-[0.14em] text-faint">404</p>
      <h1 className="mt-3 font-display text-[40px] leading-none text-foreground">
        This page does not exist
      </h1>
      <Link to="/" className="mt-5 text-[12.5px] text-brand-ink underline-offset-4 hover:underline">
        Back to the overview
      </Link>
    </div>
  );
}

function guarded(component: React.ComponentType, permission: Permission) {
  return function PermissionGuardedPage() {
    return (
      <PermissionGate permission={permission}>{React.createElement(component)}</PermissionGate>
    );
  };
}

const rootRoute = createRootRoute({
  component: RootLayout,
  notFoundComponent: NotFoundPage,
});

const overviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: OverviewPage,
});

const ordersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/orders",
  component: guarded(OrdersPage, PERMISSIONS.orders.read),
});

const productsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/products",
  component: guarded(ProductsPage, PERMISSIONS.products.read),
});

const inventoryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/inventory",
  component: guarded(InventoryPage, PERMISSIONS.inventory.read),
});

const customersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/customers",
  component: guarded(CustomersPage, PERMISSIONS.customers.read),
});

const promotionsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/promotions",
  component: guarded(PromotionsPage, PERMISSIONS.promotions.read),
});

const formsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/forms",
  component: guarded(FormsPage, PERMISSIONS.settings.read),
});

const formEditorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/forms/$formId",
  component: FormEditorPage,
});

const inboxRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/inbox",
  component: guarded(InboxPage, PERMISSIONS.settings.read),
});

const notificationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/notifications",
  component: guarded(NotificationsPage, PERMISSIONS.settings.read),
});

const themesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/themes",
  component: guarded(ThemesPage, PERMISSIONS.themes.read),
});

const themeDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/themes/$themeId",
  component: ThemeDetailPage,
});

const pagesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/pages",
  component: guarded(PagesPage, PERMISSIONS.themes.read),
});

const pageEditorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/pages/$pageId",
  component: PageEditorPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: guarded(SettingsPage, PERMISSIONS.settings.read),
});

const regionsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/regions",
  component: guarded(RegionsPage, PERMISSIONS.settings.read),
});

const shippingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/shipping",
  component: guarded(ShippingPage, PERMISSIONS.settings.read),
});

const taxRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tax",
  component: guarded(TaxPage, PERMISSIONS.settings.read),
});

const routeTree = rootRoute.addChildren([
  overviewRoute,
  ordersRoute,
  productsRoute,
  inventoryRoute,
  customersRoute,
  promotionsRoute,
  formsRoute,
  formEditorRoute,
  inboxRoute,
  notificationsRoute,
  themesRoute,
  themeDetailRoute,
  pagesRoute,
  pageEditorRoute,
  settingsRoute,
  regionsRoute,
  shippingRoute,
  taxRoute,
]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  defaultNotFoundComponent: NotFoundPage,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
