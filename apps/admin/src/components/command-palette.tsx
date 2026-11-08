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
import { requestCommand, type AdminCommand } from "@/lib/commands";
import { useTheme } from "@/components/theme-provider";

const PAGES = [
  { to: "/", label: "Overview", icon: LayoutGrid },
  { to: "/orders", label: "Orders", icon: ReceiptText },
  { to: "/products", label: "Products", icon: Package },
  { to: "/inventory", label: "Inventory", icon: Layers3 },
  { to: "/customers", label: "Customers", icon: Users },
  { to: "/promotions", label: "Promotions", icon: TicketPercent },
  { to: "/pages", label: "Pages", icon: LayoutTemplate },
  { to: "/themes", label: "Themes", icon: Palette },
  { to: "/regions", label: "Regions", icon: Globe2 },
  { to: "/shipping", label: "Shipping", icon: Truck },
  { to: "/tax", label: "Tax", icon: Percent },
  { to: "/settings", label: "Settings", icon: Settings2 },
] as const;

export function CommandPalette({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const { store, logout, mode } = useAuth();
  const { toggleTheme } = useTheme();

  const go = (to: (typeof PAGES)[number]["to"], command?: AdminCommand) => {
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
          {PAGES.map((page) => {
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
