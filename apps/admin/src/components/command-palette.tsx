import { useNavigate } from "@tanstack/react-router";
import {
  ExternalLink,
  Globe2,
  Layers3,
  LayoutGrid,
  LayoutTemplate,
  LogOut,
  Package,
  Palette,
  Percent,
  Plus,
  ReceiptText,
  Settings2,
  SlidersHorizontal,
  SunMoon,
  TicketPercent,
  Truck,
  Users,
} from "lucide-react";

import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "@/components/ui/command";
import { useAuth } from "@/lib/auth";
import { PERMISSIONS, type Permission } from "@/lib/permissions";
import { requestCommand, type AdminCommand } from "@/lib/commands";
import { useTheme } from "@/components/theme-provider";

const PAGES: Array<{
  to: string;
  label: string;
  icon: typeof LayoutGrid;
  permission?: Permission;
}> = [
  { to: "/", label: "Overview", icon: LayoutGrid },
  { to: "/orders", label: "Orders", icon: ReceiptText, permission: PERMISSIONS.orders.read },
  { to: "/products", label: "Products", icon: Package, permission: PERMISSIONS.products.read },
  { to: "/inventory", label: "Inventory", icon: Layers3, permission: PERMISSIONS.inventory.read },
  { to: "/customers", label: "Customers", icon: Users, permission: PERMISSIONS.customers.read },
  {
    to: "/promotions",
    label: "Promotions",
    icon: TicketPercent,
    permission: PERMISSIONS.promotions.read,
  },
  { to: "/pages", label: "Pages", icon: LayoutTemplate, permission: PERMISSIONS.themes.read },
  { to: "/themes", label: "Themes", icon: Palette, permission: PERMISSIONS.themes.read },
  { to: "/regions", label: "Regions", icon: Globe2, permission: PERMISSIONS.settings.read },
  { to: "/shipping", label: "Shipping", icon: Truck, permission: PERMISSIONS.settings.read },
  { to: "/tax", label: "Tax", icon: Percent, permission: PERMISSIONS.settings.read },
  { to: "/settings", label: "Settings", icon: Settings2, permission: PERMISSIONS.settings.read },
];

export function CommandPalette({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const { store, logout, mode, can } = useAuth();
  const { toggleTheme } = useTheme();

  const go = (to: string, command?: AdminCommand) => {
    onOpenChange(false);
    if (command) requestCommand(command);
    void navigate({ to });
  };

  return (
    <CommandDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Command menu"
      description="Search pages and actions"
    >
      <CommandInput placeholder="Search pages and actions…" />
      <CommandList>
        <CommandEmpty>No matches.</CommandEmpty>
        <CommandGroup heading="Go to">
          {PAGES.filter((page) => !page.permission || can(page.permission)).map((page) => {
            const Icon = page.icon;
            return (
              <CommandItem
                key={page.to}
                value={`${page.label} ${page.to}`}
                onSelect={() => go(page.to)}
              >
                <Icon aria-hidden="true" />
                {page.label}
              </CommandItem>
            );
          })}
        </CommandGroup>
        <CommandSeparator />
        <CommandGroup heading="Actions">
          <CommandItem value="new product add" onSelect={() => go("/products", "new-product")}>
            <Plus aria-hidden="true" />
            Add product
          </CommandItem>
          <CommandItem
            value="new promotion discount coupon"
            onSelect={() => go("/promotions", "new-promotion")}
          >
            <Plus aria-hidden="true" />
            Create promotion
          </CommandItem>
          <CommandItem
            value="adjust stock inventory"
            onSelect={() => go("/inventory", "adjust-stock")}
          >
            <SlidersHorizontal aria-hidden="true" />
            Adjust stock
          </CommandItem>
          <CommandItem
            value="new region country currency market"
            onSelect={() => go("/regions", "new-region")}
          >
            <Plus aria-hidden="true" />
            Create region
          </CommandItem>
          <CommandItem
            value="new shipping option flat rate delivery"
            onSelect={() => go("/shipping", "new-shipping-option")}
          >
            <Plus aria-hidden="true" />
            Create shipping option
          </CommandItem>
          <CommandItem value="new tax rate percentage" onSelect={() => go("/tax", "new-tax-rate")}>
            <Plus aria-hidden="true" />
            Create tax rate
          </CommandItem>
          <CommandItem value="theme dark light" onSelect={toggleTheme}>
            <SunMoon aria-hidden="true" />
            Switch theme
          </CommandItem>
          {store?.primary_domain ? (
            <CommandItem
              value="storefront open website"
              onSelect={() => {
                onOpenChange(false);
                window.open(`https://${store.primary_domain}`, "_blank", "noopener");
              }}
            >
              <ExternalLink aria-hidden="true" />
              Open storefront
            </CommandItem>
          ) : null}
          <CommandItem
            value="sign out log out"
            onSelect={() => {
              onOpenChange(false);
              void logout();
            }}
          >
            <LogOut aria-hidden="true" />
            {mode === "demo" ? "Exit demo" : "Sign out"}
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
