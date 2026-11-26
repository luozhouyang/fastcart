import React, { Suspense } from "react";
import {
  Link,
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
} from "@tanstack/react-router";

import { AppShell } from "@/components/app-shell";
import { ThemeProvider, useTheme } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { PermissionGate } from "@/components/permission-gate";
import { PERMISSIONS, type Permission } from "@/lib/permissions";
import { AuthProvider, useAuth } from "@/lib/auth";
import { LoginPage, NoStorePage, SplashScreen } from "@/pages/login";
import { OverviewPage } from "@/pages/overview";
import { PluginExtensionsProvider } from "@/plugins/runtime";

const CustomersPage = lazyRouteComponent(() =>
  import("@/pages/customers").then((module) => ({ default: module.CustomersPage })),
);
const FormEditorPage = lazyRouteComponent(() =>
  import("@/pages/form-editor").then((module) => ({ default: module.FormEditorPage })),
);
const FormsPage = lazyRouteComponent(() =>
  import("@/pages/forms").then((module) => ({ default: module.FormsPage })),
);
const InboxPage = lazyRouteComponent(() =>
  import("@/pages/inbox").then((module) => ({ default: module.InboxPage })),
);
const InventoryPage = lazyRouteComponent(() =>
  import("@/pages/inventory").then((module) => ({ default: module.InventoryPage })),
);
const NotificationsPage = lazyRouteComponent(() =>
  import("@/pages/notifications").then((module) => ({ default: module.NotificationsPage })),
);
const OrdersPage = lazyRouteComponent(() =>
  import("@/pages/orders").then((module) => ({ default: module.OrdersPage })),
);
const PageEditorPage = lazyRouteComponent(() =>
  import("@/pages/page-editor").then((module) => ({ default: module.PageEditorPage })),
);
const PagesPage = lazyRouteComponent(() =>
  import("@/pages/pages").then((module) => ({ default: module.PagesPage })),
);
const ProductsPage = lazyRouteComponent(() =>
  import("@/pages/products").then((module) => ({ default: module.ProductsPage })),
);
const PromotionsPage = lazyRouteComponent(() =>
  import("@/pages/promotions").then((module) => ({ default: module.PromotionsPage })),
);
const RegionsPage = lazyRouteComponent(() =>
  import("@/pages/regions").then((module) => ({ default: module.RegionsPage })),
);
const SettingsPage = lazyRouteComponent(() =>
  import("@/pages/settings").then((module) => ({ default: module.SettingsPage })),
);
const ShippingPage = lazyRouteComponent(() =>
  import("@/pages/shipping").then((module) => ({ default: module.ShippingPage })),
);
const TaxPage = lazyRouteComponent(() =>
  import("@/pages/tax").then((module) => ({ default: module.TaxPage })),
);
const ThemeDetailPage = lazyRouteComponent(() =>
  import("@/pages/theme-detail").then((module) => ({ default: module.ThemeDetailPage })),
);
const ThemesPage = lazyRouteComponent(() =>
  import("@/pages/themes").then((module) => ({ default: module.ThemesPage })),
);
const PluginsPage = lazyRouteComponent(() =>
  import("@/pages/plugins").then((module) => ({ default: module.PluginsPage })),
);
const CapabilitiesPage = lazyRouteComponent(() =>
  import("@/pages/capabilities").then((module) => ({ default: module.CapabilitiesPage })),
);
const PluginPage = lazyRouteComponent(() =>
  import("@/pages/plugin-page").then((module) => ({ default: module.PluginPage })),
);
const PluginDetailPage = lazyRouteComponent(() =>
  import("@/pages/plugin-detail").then((module) => ({ default: module.PluginDetailPage })),
);
const CustomerGroupsPage = lazyRouteComponent(() =>
  import("@/pages/customer-groups").then((module) => ({ default: module.CustomerGroupsPage })),
);
const TaxonomyPage = lazyRouteComponent(() =>
  import("@/pages/taxonomy").then((module) => ({ default: module.TaxonomyPage })),
);
const MediaPage = lazyRouteComponent(() =>
  import("@/pages/media").then((module) => ({ default: module.MediaPage })),
);
const PriceListsPage = lazyRouteComponent(() =>
  import("@/pages/price-lists").then((module) => ({ default: module.PriceListsPage })),
);
const CampaignsPage = lazyRouteComponent(() =>
  import("@/pages/campaigns").then((module) => ({ default: module.CampaignsPage })),
);
const WebhooksPage = lazyRouteComponent(() =>
  import("@/pages/webhooks").then((module) => ({ default: module.WebhooksPage })),
);
const PaymentsPage = lazyRouteComponent(() =>
  import("@/pages/payments").then((module) => ({ default: module.PaymentsPage })),
);
const GiftCardsPage = lazyRouteComponent(() =>
  import("@/pages/gift-cards").then((module) => ({ default: module.GiftCardsPage })),
);
const ReturnReasonsPage = lazyRouteComponent(() =>
  import("@/pages/return-reasons").then((module) => ({ default: module.ReturnReasonsPage })),
);

function PageLoading() {
  return (
    <div className="flex min-h-[40vh] items-center justify-center" role="status" aria-live="polite">
      <span className="size-6 animate-spin rounded-full border-2 border-border border-t-brand" />
      <span className="sr-only">Loading page</span>
    </div>
  );
}

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
        <PluginExtensionsProvider>
          <AppGate />
        </PluginExtensionsProvider>
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
      <PermissionGate permission={permission}>
        <Suspense fallback={<PageLoading />}>{React.createElement(component)}</Suspense>
      </PermissionGate>
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

const pluginsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/plugins",
  component: guarded(PluginsPage, PERMISSIONS.plugins.read),
});

const capabilitiesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/capabilities",
  component: guarded(CapabilitiesPage, PERMISSIONS.plugins.read),
});

const pluginPageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps/$pluginId",
  component: PluginPage,
});

const pluginPageNestedRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/apps/$pluginId/$",
  component: PluginPage,
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

const pluginDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/plugins/$pluginId",
  component: guarded(PluginDetailPage, PERMISSIONS.plugins.read),
});

const customerGroupsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/customer-groups",
  component: guarded(CustomerGroupsPage, PERMISSIONS.customers.read),
});

const taxonomyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/taxonomy",
  component: guarded(TaxonomyPage, PERMISSIONS.products.read),
});

const mediaRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/media",
  component: guarded(MediaPage, PERMISSIONS.products.read),
});

const priceListsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/price-lists",
  component: guarded(PriceListsPage, PERMISSIONS.promotions.read),
});

const campaignsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/campaigns",
  component: guarded(CampaignsPage, PERMISSIONS.promotions.read),
});

const webhooksRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/webhooks",
  component: guarded(WebhooksPage, PERMISSIONS.settings.read),
});

const paymentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/payments",
  component: guarded(PaymentsPage, PERMISSIONS.payments.read),
});

const giftCardsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/gift-cards",
  component: guarded(GiftCardsPage, PERMISSIONS.payments.read),
});

const returnReasonsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/return-reasons",
  component: guarded(ReturnReasonsPage, PERMISSIONS.orders.read),
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
  pluginsRoute,
  capabilitiesRoute,
  pluginDetailRoute,
  pluginPageRoute,
  pluginPageNestedRoute,
  customerGroupsRoute,
  taxonomyRoute,
  mediaRoute,
  priceListsRoute,
  campaignsRoute,
  webhooksRoute,
  paymentsRoute,
  giftCardsRoute,
  returnReasonsRoute,
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
