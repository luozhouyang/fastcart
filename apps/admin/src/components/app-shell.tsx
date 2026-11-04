import { useEffect, useMemo, useState } from "react";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import {
  ArrowUpRight,
  Command,
  Bell,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsRight,
  ChevronsUpDown,
  FileText,
  Inbox,
  Layers3,
  LayoutGrid,
  LogOut,
  Menu,
  Package,
  ReceiptText,
  Search,
  Settings as SettingsIcon,
  Settings2,
  SunMoon,
  Users,
} from "lucide-react";

import { CommandPalette } from "@/components/command-palette";
import { ThemeToggle } from "@/components/theme-toggle";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { demoNotifications } from "@/lib/demo";
import { demoUnreadInquiries } from "@/lib/demo-inquiries";
import { useAuth } from "@/lib/auth";
import { initials } from "@/lib/format";
import type { NotificationView } from "@/lib/models";
import { useTheme } from "@/components/theme-provider";
import { cn } from "@/lib/utils";

interface NavItem {
  to:
    | "/"
    | "/orders"
    | "/products"
    | "/inventory"
    | "/customers"
    | "/forms"
    | "/inbox"
    | "/notifications"
    | "/settings";
  label: string;
  icon: typeof LayoutGrid;
}

const NAV_GROUPS: Array<{ label: string; items: NavItem[] }> = [
  {
    label: "Workspace",
    items: [{ to: "/", label: "Overview", icon: LayoutGrid }],
  },
  {
    label: "Operations",
    items: [
      { to: "/orders", label: "Orders", icon: ReceiptText },
      { to: "/products", label: "Products", icon: Package },
      { to: "/inventory", label: "Inventory", icon: Layers3 },
      { to: "/customers", label: "Customers", icon: Users },
      { to: "/forms", label: "Forms", icon: FileText },
      { to: "/inbox", label: "Inbox", icon: Inbox },
    ],
  },
  {
    label: "Configuration",
    items: [
      { to: "/notifications", label: "Notifications", icon: Bell },
      { to: "/settings", label: "Settings", icon: SettingsIcon },
    ],
  },
];

const PAGE_TITLES: Record<string, string> = {
  "/": "Overview",
  "/orders": "Orders",
  "/products": "Products",
  "/inventory": "Inventory",
  "/customers": "Customers",
  "/forms": "Forms",
  "/inbox": "Inbox",
  "/notifications": "Notifications",
  "/settings": "Settings",
};

function pageTitleFor(pathname: string): string {
  const title = PAGE_TITLES[pathname];
  if (title) return title;
  if (pathname.startsWith("/forms/")) return "Form editor";
  return "Overview";
}

function Wordmark({ compact = false }: { compact?: boolean }) {
  return (
    <Link
      to="/"
      className="inline-flex items-center gap-2.5 rounded-lg outline-none focus-visible:ring-[3px] focus-visible:ring-ring/30"
      aria-label="Fastcart admin home"
    >
      <span className="flex size-8 items-center justify-center rounded-[9px] bg-primary text-primary-foreground">
        <ChevronsRight className="size-4" strokeWidth={2.4} aria-hidden="true" />
      </span>
      {compact ? null : (
        <span className="text-[17px] font-medium tracking-[-0.03em] text-foreground">fastcart</span>
      )}
    </Link>
  );
}

function useOrderSignal(): number {
  const { mode, client, store, dataKey } = useAuth();
  const [pending, setPending] = useState(0);

  useEffect(() => {
    if (mode === "demo") {
      setPending(12);
      return;
    }
    if (!client || !store) {
      setPending(0);
      return;
    }
    let active = true;
    client.orders
      .list({ status: "pending", limit: 1 })
      .then((page) => {
        if (active) setPending(page.pagination.total);
      })
      .catch(() => {
        if (active) setPending(0);
      });
    return () => {
      active = false;
    };
  }, [mode, client, store, dataKey]);

  return pending;
}

function useInquirySignal(): number {
  const { mode, client, store, dataKey } = useAuth();
  const [unread, setUnread] = useState(0);

  useEffect(() => {
    if (mode === "demo") {
      setUnread(demoUnreadInquiries());
      return;
    }
    if (!client || !store) {
      setUnread(0);
      return;
    }
    let active = true;
    client.inquiries
      .unreadCount()
      .then((result) => {
        if (!active) return;
        const record = result as unknown as { count?: unknown; unread?: unknown };
        const value =
          typeof record.count === "number"
            ? record.count
            : typeof record.unread === "number"
              ? record.unread
              : 0;
        setUnread(value);
      })
      .catch(() => {
        if (active) setUnread(0);
      });
    return () => {
      active = false;
    };
  }, [mode, client, store, dataKey]);

  return unread;
}

function SidebarBody({
  pending,
  unread,
  onNavigate,
}: {
  pending: number;
  unread: number;
  onNavigate?: () => void;
}) {
  const { mode, stores, store, selectStore } = useAuth();
  const pathname = useRouterState({ select: (state) => state.location.pathname });

  return (
    <div className="flex h-full flex-col">
      <div className="px-5 pt-5">
        <Wordmark />
      </div>

      <div className="px-4 pt-5">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex w-full cursor-pointer items-center gap-3 rounded-xl border border-border bg-card p-3 text-left shadow-soft outline-none transition-colors duration-150 hover:border-line-strong focus-visible:ring-[3px] focus-visible:ring-ring/30"
            >
              <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-brand-soft font-display text-[17px] text-brand-ink">
                {(store?.name ?? "S").charAt(0).toUpperCase()}
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] leading-tight font-medium text-foreground">
                  {store?.name ?? "No store selected"}
                </span>
                <span className="mt-1 block text-[11px] text-muted-foreground">
                  {mode === "demo" ? "Demo store" : "Active store"}
                </span>
              </span>
              <ChevronsUpDown className="size-3.5 shrink-0 text-faint" aria-hidden="true" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-[236px]">
            <DropdownMenuLabel>Stores</DropdownMenuLabel>
            {stores.map((entry) => (
              <DropdownMenuItem
                key={entry.id}
                onSelect={() => {
                  selectStore(entry.id);
                  onNavigate?.();
                }}
                className="gap-2.5"
              >
                <span className="flex size-6 items-center justify-center rounded-md bg-secondary font-display text-[12px] text-ink-soft">
                  {entry.name.charAt(0).toUpperCase()}
                </span>
                <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                {entry.id === store?.id ? (
                  <Check className="size-3.5 text-brand" aria-hidden="true" />
                ) : null}
              </DropdownMenuItem>
            ))}
            {mode === "demo" ? (
              <p className="px-2.5 pt-2 pb-1.5 text-[11px] leading-relaxed text-faint">
                Reviewing the design dataset. Sign in to connect a live store.
              </p>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <nav className="mt-6 flex-1 overflow-y-auto px-3 pb-4" aria-label="Admin sections">
        {NAV_GROUPS.map((group) => (
          <div key={group.label} className="mb-5">
            <p className="px-3 pb-1.5 text-[11.5px] font-medium text-muted-foreground">
              {group.label}
            </p>
            <ul className="space-y-0.5">
              {group.items.map((item) => {
                const active = item.to === "/" ? pathname === "/" : pathname.startsWith(item.to);
                const Icon = item.icon;
                return (
                  <li key={item.to}>
                    <Link
                      to={item.to}
                      onClick={onNavigate}
                      aria-current={active ? "page" : undefined}
                      className={cn(
                        "relative flex h-9 items-center gap-2.5 rounded-lg px-3 text-[13px] transition-colors duration-150",
                        "outline-none focus-visible:ring-[3px] focus-visible:ring-ring/30",
                        active
                          ? "bg-accent font-medium text-foreground"
                          : "text-ink-soft hover:bg-accent/70 hover:text-foreground",
                      )}
                    >
                      {active ? (
                        <span
                          className="absolute inset-y-2 left-0 w-[2px] rounded-full bg-brand"
                          aria-hidden="true"
                        />
                      ) : null}
                      <Icon
                        className={cn(
                          "size-[17px] shrink-0",
                          active ? "text-foreground" : "text-faint",
                        )}
                        strokeWidth={1.8}
                        aria-hidden="true"
                      />
                      <span className="flex-1">{item.label}</span>
                      {item.to === "/orders" && pending > 0 ? (
                        <span className="rounded-full bg-secondary px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground tnum">
                          {pending}
                        </span>
                      ) : item.to === "/inbox" && unread > 0 ? (
                        <span className="rounded-full bg-brand-soft px-1.5 py-0.5 font-mono text-[10px] text-brand-ink tnum">
                          {unread}
                        </span>
                      ) : null}
                    </Link>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </nav>

      <div className="px-4 pb-3">
        <div className="rounded-xl border border-border bg-card p-3.5 shadow-soft">
          <div className="flex items-center justify-between">
            <span className="text-[11px] font-medium text-muted-foreground">Storefront</span>
            <span className="inline-flex items-center gap-1.5 text-[10.5px] font-medium text-brand-ink">
              <span className="size-1.5 rounded-full bg-brand" aria-hidden="true" />
              Live
            </span>
          </div>
          {store?.primary_domain ? (
            <a
              href={`https://${store.primary_domain}`}
              target="_blank"
              rel="noreferrer"
              className="mt-2 flex items-center justify-between gap-2 text-[12px] text-foreground hover:text-brand-ink"
            >
              <span className="truncate">{store.primary_domain}</span>
              <ArrowUpRight className="size-3.5 shrink-0 text-faint" aria-hidden="true" />
            </a>
          ) : (
            <p className="mt-2 text-[12px] text-faint">No domain connected</p>
          )}
        </div>
        <div className="mt-3 flex items-center justify-between px-1 text-[10.5px] text-faint">
          <span className="font-mono">v0.1.0</span>
          <span>Self-hosted</span>
        </div>
      </div>
    </div>
  );
}

function Notifications({ pending }: { pending: number }) {
  const { mode, store } = useAuth();
  const [read, setRead] = useState(false);

  const notifications = useMemo<NotificationView[]>(() => {
    if (mode === "demo") return demoNotifications;
    if (pending > 0) {
      return [
        {
          id: "pending-orders",
          title: `${pending} ${pending === 1 ? "order" : "orders"} awaiting payment`,
          body: "Payment has not been captured yet.",
          time: "Live",
          tone: "warning",
        },
      ];
    }
    return [];
  }, [mode, pending]);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="relative rounded-full text-muted-foreground"
          aria-label="Notifications"
        >
          <Bell className="size-[17px]" />
          {notifications.length > 0 && !read ? (
            <span className="absolute top-2 right-2.5 size-1.5 rounded-full bg-brand ring-2 ring-background" />
          ) : null}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-[320px] p-0">
        <div className="flex items-center justify-between px-4 py-3">
          <span className="text-[12.5px] font-medium text-foreground">Notifications</span>
          {notifications.length > 0 ? (
            <button
              type="button"
              onClick={() => setRead(true)}
              className="cursor-pointer text-[11px] text-muted-foreground hover:text-foreground"
            >
              Mark as read
            </button>
          ) : null}
        </div>
        <DropdownMenuSeparator className="my-0" />
        {notifications.length === 0 ? (
          <p className="px-4 py-6 text-center text-[12px] text-muted-foreground">
            You are all caught up.
          </p>
        ) : (
          <ul className="max-h-[320px] overflow-y-auto p-1.5">
            {notifications.map((notification) => (
              <li key={notification.id} className="rounded-lg px-2.5 py-2.5 hover:bg-accent">
                <div className="flex items-start gap-2.5">
                  <span
                    className={cn(
                      "mt-1.5 size-1.5 shrink-0 rounded-full",
                      notification.tone === "warning"
                        ? "bg-warning"
                        : notification.tone === "info"
                          ? "bg-info"
                          : "bg-brand",
                    )}
                    aria-hidden="true"
                  />
                  <div className="min-w-0 flex-1">
                    <p className="text-[12.5px] font-medium text-foreground">
                      {notification.title}
                    </p>
                    <p className="mt-0.5 text-[11.5px] leading-relaxed text-muted-foreground">
                      {notification.body}
                    </p>
                    <p className="mt-1.5 font-mono text-[10px] text-faint">{notification.time}</p>
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
        <DropdownMenuSeparator className="my-0" />
        <div className="px-4 py-2.5 text-[11px] text-faint">
          {mode === "demo" ? "Demo notifications" : `Store ${store?.name ?? ""}`}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function UserMenu() {
  const { user, mode, logout } = useAuth();
  const { toggleTheme } = useTheme();
  const navigate = useNavigate();

  if (!user) return null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex cursor-pointer items-center gap-2.5 rounded-full p-0.5 pr-1 outline-none transition-colors hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/30 sm:pr-2.5"
          aria-label="Account menu"
        >
          <Avatar className="size-8">
            <AvatarFallback className="text-[10.5px]">{initials(user.name)}</AvatarFallback>
          </Avatar>
          <span className="hidden text-left sm:block">
            <span className="block text-[12px] leading-tight font-medium text-foreground">
              {user.name}
            </span>
            <span className="mt-0.5 block text-[10.5px] text-muted-foreground">{user.role}</span>
          </span>
          <ChevronDown className="hidden size-3.5 text-faint sm:block" aria-hidden="true" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-[240px]">
        <DropdownMenuLabel>
          <span className="block text-[12px] font-medium text-foreground">{user.name}</span>
          <span className="mt-0.5 block truncate text-[11px] font-normal text-muted-foreground">
            {user.email}
          </span>
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => void navigate({ to: "/settings" })}>
          <Settings2 aria-hidden="true" />
          Store settings
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={toggleTheme}>
          <SunMoon aria-hidden="true" />
          Switch theme
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onSelect={() => void logout()}>
          <LogOut aria-hidden="true" />
          {mode === "demo" ? "Exit demo" : "Sign out"}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function AppShell() {
  const { store, mode } = useAuth();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const [commandOpen, setCommandOpen] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const pending = useOrderSignal();
  const unread = useInquirySignal();

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const pageTitle = pageTitleFor(pathname);

  return (
    <div className="min-h-screen bg-background">
      <aside className="fixed inset-y-0 left-0 z-40 hidden w-[272px] border-r border-border bg-card/55 lg:flex lg:flex-col">
        <SidebarBody pending={pending} unread={unread} />
      </aside>

      <div className="lg:pl-[272px]">
        <header className="sticky top-0 z-30 flex h-16 items-center gap-3 border-b border-border bg-background/88 px-4 backdrop-blur-md sm:px-6 lg:px-10">
          <Button
            variant="ghost"
            size="icon"
            className="rounded-full text-muted-foreground lg:hidden"
            onClick={() => setMobileOpen(true)}
            aria-label="Open navigation"
          >
            <Menu className="size-[18px]" />
          </Button>

          <div className="hidden min-w-0 items-center gap-2 text-[12.5px] md:flex">
            <span className="truncate text-muted-foreground">{store?.name}</span>
            <ChevronRight className="size-3.5 shrink-0 text-faint" aria-hidden="true" />
            <span className="text-foreground">{pageTitle}</span>
            {mode === "demo" ? (
              <span className="ml-1 rounded-full border border-border bg-card px-2 py-0.5 text-[10.5px] font-medium text-muted-foreground">
                Demo
              </span>
            ) : null}
          </div>

          <div className="flex flex-1 items-center justify-end gap-1.5">
            <button
              type="button"
              onClick={() => setCommandOpen(true)}
              className="mr-1 hidden h-9 cursor-pointer items-center gap-2 rounded-full border border-border bg-card pr-1.5 pl-3 text-[12px] text-faint shadow-xs transition-colors hover:border-line-strong focus-visible:ring-[3px] focus-visible:ring-ring/30 focus-visible:outline-none lg:flex lg:w-[300px]"
            >
              <Search className="size-3.5" aria-hidden="true" />
              <span className="flex-1 truncate text-left">Search orders, products…</span>
              <kbd className="flex items-center gap-0.5 rounded-[6px] border border-border bg-secondary px-1.5 py-0.5 text-[10px] text-muted-foreground">
                <Command className="size-2.5" aria-hidden="true" />K
              </kbd>
            </button>
            <Button
              variant="ghost"
              size="icon"
              className="rounded-full text-muted-foreground lg:hidden"
              onClick={() => setCommandOpen(true)}
              aria-label="Search"
            >
              <Search className="size-[18px]" />
            </Button>
            <ThemeToggle />
            <Notifications pending={pending} />
            <div className="mx-1 hidden h-5 w-px bg-border sm:block" aria-hidden="true" />
            <UserMenu />
          </div>
        </header>

        <main className="mx-auto w-full max-w-[1184px] px-5 pt-8 pb-16 sm:px-8 lg:px-10 lg:pt-9">
          <Outlet />
        </main>
      </div>

      <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
        <SheetContent side="left" className="w-[288px] p-0" showCloseButton={false}>
          <SheetTitle className="sr-only">Navigation</SheetTitle>
          <SidebarBody pending={pending} unread={unread} onNavigate={() => setMobileOpen(false)} />
        </SheetContent>
      </Sheet>

      <CommandPalette open={commandOpen} onOpenChange={setCommandOpen} />
    </div>
  );
}
