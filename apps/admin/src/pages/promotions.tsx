import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  ChevronDown,
  Eye,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Sparkles,
  TicketPercent,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import type {
  PageResult,
  Promotion,
  PromotionListQuery,
  PromotionStatus,
  PromotionType,
  UpdatePromotionRequest,
} from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
import { ProgressBar } from "@/components/progress-bar";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { consumePendingCommand, onCommand } from "@/lib/commands";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  couponCodeError,
  formatPromotionValue,
  normalizeCouponCode,
  parsePromotionValue,
  PROMOTION_STATUS_LABELS,
  PROMOTION_STATUS_TONES,
  PROMOTION_STATUSES,
  PROMOTION_TYPE_LABELS,
  PROMOTION_TYPES,
  promotionChannelLabel,
  promotionRuleSummary,
  promotionUsageLabel,
  promotionValueToInput,
  type PromotionStatusTone,
} from "@/lib/promotions";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 10;

const STATUS_FILTERS: Array<{ value: PromotionStatus | "all"; label: string }> = [
  { value: "all", label: "All" },
  ...PROMOTION_STATUSES.map((status) => ({
    value: status,
    label: PROMOTION_STATUS_LABELS[status],
  })),
];

const STATUS_DOTS: Record<PromotionStatusTone, string> = {
  brand: "bg-brand",
  secondary: "bg-faint",
  warning: "bg-warning",
  destructive: "bg-destructive",
};

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70";

interface CouponRef {
  id: string;
  code: string;
}

interface PromotionDraft {
  title: string;
  promotionType: PromotionType;
  value: number;
  status: PromotionStatus;
  isAutomatic: boolean;
  couponCode: string;
}

const DEMO_CLOCK = Math.floor(new Date("2026-11-08T12:00:00Z").getTime() / 1000);

function demoPromotionSeed(): Promotion[] {
  const day = 86_400;
  const created = (daysAgo: number) => DEMO_CLOCK - daysAgo * day;

  return [
    {
      id: "promo_first20",
      code: null,
      title: "$20 off a first order",
      promotion_type: "fixed_amount",
      value: 2_000,
      status: "draft",
      is_automatic: false,
      starts_at: null,
      ends_at: null,
      usage_limit: 50,
      usage_count: 0,
      metadata: {},
      rules: [
        {
          id: "rule_first20",
          promotion_id: "promo_first20",
          attribute: "first_order",
          operator: "eq",
          value: "true",
          created_at: created(2),
        },
      ],
      created_at: created(2),
      updated_at: created(2),
    },
    {
      id: "promo_summer",
      code: null,
      title: "Summer studio edit",
      promotion_type: "percentage",
      value: 1_500,
      status: "active",
      is_automatic: false,
      starts_at: created(20),
      ends_at: DEMO_CLOCK + 10 * day,
      usage_limit: 500,
      usage_count: 42,
      metadata: {},
      rules: [
        {
          id: "rule_summer",
          promotion_id: "promo_summer",
          attribute: "minimum_order_subtotal",
          operator: "gte",
          value: "7500",
          created_at: created(3),
        },
      ],
      created_at: created(3),
      updated_at: created(3),
    },
    {
      id: "promo_ship",
      code: null,
      title: "Free shipping over $75",
      promotion_type: "free_shipping",
      value: 0,
      status: "active",
      is_automatic: true,
      starts_at: null,
      ends_at: null,
      usage_limit: null,
      usage_count: 128,
      metadata: {},
      rules: [
        {
          id: "rule_ship",
          promotion_id: "promo_ship",
          attribute: "minimum_order_subtotal",
          operator: "gte",
          value: "7500",
          created_at: created(10),
        },
      ],
      created_at: created(10),
      updated_at: created(10),
    },
    {
      id: "promo_welcome",
      code: "WELCOME15",
      title: "Welcome offer",
      promotion_type: "percentage",
      value: 1_000,
      status: "active",
      is_automatic: false,
      starts_at: created(30),
      ends_at: null,
      usage_limit: 100,
      usage_count: 37,
      metadata: {},
      rules: [],
      created_at: created(24),
      updated_at: created(24),
    },
    {
      id: "promo_spring",
      code: "SPRING25",
      title: "Spring clearance",
      promotion_type: "percentage",
      value: 2_500,
      status: "expired",
      is_automatic: false,
      starts_at: created(70),
      ends_at: created(30),
      usage_limit: null,
      usage_count: 210,
      metadata: {},
      rules: [],
      created_at: created(60),
      updated_at: created(30),
    },
    {
      id: "promo_vip",
      code: null,
      title: "VIP appreciation",
      promotion_type: "fixed_amount",
      value: 5_000,
      status: "disabled",
      is_automatic: false,
      starts_at: null,
      ends_at: null,
      usage_limit: null,
      usage_count: 8,
      metadata: {},
      rules: [],
      created_at: created(90),
      updated_at: created(90),
    },
  ];
}

function demoCouponSeed(): Record<string, string> {
  return {
    promo_first20: "FIRST20",
    promo_summer: "SUMMER15",
    promo_welcome: "WELCOME15",
    promo_spring: "SPRING25",
    promo_vip: "VIP50",
  };
}

export function PromotionsPage() {
  const { mode, client, store, dataKey } = useAuth();
  const [statusFilter, setStatusFilter] = useState<PromotionStatus | "all">("all");
  const [page, setPage] = useState(1);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<Promotion | null>(null);
  const [editingCoupon, setEditingCoupon] = useState<CouponRef | null>(null);
  const [couponLoading, setCouponLoading] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<Promotion | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [actionBusyId, setActionBusyId] = useState<string | null>(null);
  const [detail, setDetail] = useState<Promotion | null>(null);
  const [detailCoupon, setDetailCoupon] = useState<CouponRef | null>(null);
  const [detailCouponLoading, setDetailCouponLoading] = useState(false);
  const [detailCouponError, setDetailCouponError] = useState<string | null>(null);
  const [detailUsage, setDetailUsage] = useState<number | null>(null);
  const [detailUsageLoading, setDetailUsageLoading] = useState(false);
  const [detailUsageError, setDetailUsageError] = useState<string | null>(null);
  const [demoPromotions, setDemoPromotions] = useState<Promotion[]>(demoPromotionSeed);
  const [demoCoupons, setDemoCoupons] = useState<Record<string, string>>(demoCouponSeed);

  const editToken = useRef(0);
  const couponAbort = useRef<AbortController | null>(null);

  const loadCoupon = useCallback(
    async (promotionId: string, signal?: AbortSignal): Promise<CouponRef | null> => {
      if (mode === "demo") {
        const code = demoCoupons[promotionId];
        return code ? { id: `coupon_${promotionId}`, code } : null;
      }
      if (!client) throw new Error("No store is selected.");
      const result = await client.coupons.list(
        { promotion_id: promotionId, limit: 1 },
        signal ? { signal } : undefined,
      );
      const coupon = result.data[0];
      return coupon ? { id: coupon.id, code: coupon.code } : null;
    },
    [mode, client, demoCoupons],
  );

  const openCreate = useCallback(() => {
    editToken.current += 1;
    couponAbort.current?.abort();
    couponAbort.current = null;
    setEditing(null);
    setEditingCoupon(null);
    setCouponLoading(false);
    setDialogOpen(true);
  }, []);

  useEffect(() => {
    const pending = consumePendingCommand();
    if (pending === "new-promotion") openCreate();
    return onCommand((command) => {
      if (command === "new-promotion") openCreate();
    });
  }, [openCreate]);

  useEffect(() => {
    setStatusFilter("all");
    setPage(1);
    setDetail(null);
    setDeleteTarget(null);
    setDialogOpen(false);
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<PageResult<Promotion>> => {
      if (mode === "demo") {
        const filtered = demoPromotions
          .filter((promotion) => statusFilter === "all" || promotion.status === statusFilter)
          .sort((a, b) => b.created_at - a.created_at || a.title.localeCompare(b.title));
        const offset = (page - 1) * PAGE_SIZE;
        return Promise.resolve({
          data: filtered.slice(offset, offset + PAGE_SIZE),
          pagination: { limit: PAGE_SIZE, offset, total: filtered.length },
        });
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      const query: PromotionListQuery = {
        limit: PAGE_SIZE,
        offset: (page - 1) * PAGE_SIZE,
      };
      if (statusFilter !== "all") query.status = statusFilter;
      return client.promotions.list(query, { signal });
    },
    [mode, client, statusFilter, page, demoPromotions],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    statusFilter,
    page,
    demoPromotions,
  ]);

  const rows = data?.data ?? [];
  const total = data?.pagination.total ?? 0;
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const currentPage = Math.min(page, pages);
  const currency = store?.currency_code ?? "USD";
  const start = rows.length === 0 ? 0 : (currentPage - 1) * PAGE_SIZE + 1;
  const end = rows.length === 0 ? 0 : start + rows.length - 1;

  useEffect(() => {
    if (page > pages) setPage(pages);
  }, [page, pages]);

  const openEdit = useCallback(
    (promotion: Promotion) => {
      setEditing(promotion);
      setEditingCoupon(null);
      setDialogOpen(true);
      couponAbort.current?.abort();

      if (mode === "demo") {
        const code = demoCoupons[promotion.id];
        setEditingCoupon(code ? { id: `coupon_${promotion.id}`, code } : null);
        setCouponLoading(false);
        return;
      }

      const controller = new AbortController();
      couponAbort.current = controller;
      const token = ++editToken.current;
      setCouponLoading(true);
      void loadCoupon(promotion.id, controller.signal)
        .then((coupon) => {
          if (controller.signal.aborted || token !== editToken.current) return;
          setEditingCoupon(coupon);
        })
        .catch((cause: unknown) => {
          if (controller.signal.aborted || token !== editToken.current) return;
          toast.error(`Could not load the existing coupon: ${errorMessage(cause)}`);
        })
        .finally(() => {
          if (controller.signal.aborted || token !== editToken.current) return;
          setCouponLoading(false);
        });
    },
    [mode, demoCoupons, loadCoupon],
  );

  const handleDialogOpenChange = (open: boolean) => {
    if (!open) {
      editToken.current += 1;
      couponAbort.current?.abort();
      couponAbort.current = null;
      setCouponLoading(false);
    }
    setDialogOpen(open);
  };

  const openDetail = useCallback(
    (promotion: Promotion) => {
      setDetail(promotion);
      setDetailCoupon(null);
      setDetailCouponError(null);
      setDetailUsage(promotion.usage_count);
      setDetailUsageError(null);
      setDetailCouponLoading(mode !== "demo");
      setDetailUsageLoading(mode !== "demo");
    },
    [mode],
  );

  const detailId = detail?.id ?? null;

  useEffect(() => {
    if (detailId === null) return;

    if (mode === "demo") {
      const code = demoCoupons[detailId];
      setDetailCoupon(code ? { id: `coupon_${detailId}`, code } : null);
      setDetailCouponError(null);
      setDetailCouponLoading(false);
      const promotion = demoPromotions.find((entry) => entry.id === detailId);
      setDetailUsage(promotion?.usage_count ?? null);
      setDetailUsageError(null);
      setDetailUsageLoading(false);
      return;
    }

    if (!client) return;
    const controller = new AbortController();
    setDetailCouponLoading(true);
    setDetailCouponError(null);
    setDetailUsageLoading(true);
    setDetailUsageError(null);

    void loadCoupon(detailId, controller.signal)
      .then((coupon) => {
        if (!controller.signal.aborted) setDetailCoupon(coupon);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setDetailCouponError(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setDetailCouponLoading(false);
      });

    void client.promotions
      .usage(detailId, { signal: controller.signal })
      .then((result) => {
        if (!controller.signal.aborted) setDetailUsage(result.usage_count);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setDetailUsageError(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setDetailUsageLoading(false);
      });

    return () => controller.abort();
  }, [detailId, mode, client, loadCoupon, demoCoupons, demoPromotions]);

  const handleSave = useCallback(
    async (draft: PromotionDraft): Promise<void> => {
      const code = normalizeCouponCode(draft.couponCode);

      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editing) {
          setDemoPromotions((current) =>
            current.map((promotion) =>
              promotion.id === editing.id
                ? {
                    ...promotion,
                    title: draft.title,
                    promotion_type: draft.promotionType,
                    value: draft.value,
                    status: draft.status,
                    is_automatic: draft.isAutomatic,
                    updated_at: now,
                  }
                : promotion,
            ),
          );
          if (code.length > 0) {
            setDemoCoupons((current) => ({ ...current, [editing.id]: code }));
          }
          toast.success("Promotion updated in the demo");
        } else {
          const created: Promotion = {
            id: `promo_demo_${Date.now()}`,
            code: null,
            title: draft.title,
            promotion_type: draft.promotionType,
            value: draft.value,
            status: draft.status,
            is_automatic: draft.isAutomatic,
            starts_at: null,
            ends_at: null,
            usage_limit: null,
            usage_count: 0,
            metadata: {},
            rules: [],
            created_at: now,
            updated_at: now,
          };
          setDemoPromotions((current) => [created, ...current]);
          if (code.length > 0) {
            setDemoCoupons((current) => ({ ...current, [created.id]: code }));
          }
          setPage(1);
          toast.success("Promotion created in the demo");
        }
        return;
      }

      if (!client) throw new Error("No store is selected.");

      if (editing) {
        const body: UpdatePromotionRequest = {
          title: draft.title,
          promotion_type: draft.promotionType,
          value: draft.value,
          status: draft.status,
          is_automatic: draft.isAutomatic,
        };
        const updated = await client.promotions.update(editing.id, body);
        if (code.length > 0) {
          try {
            if (editingCoupon) {
              if (normalizeCouponCode(editingCoupon.code) !== code) {
                await client.coupons.update(editingCoupon.id, { code });
              }
            } else {
              await client.coupons.create({ code, promotion_id: editing.id });
            }
          } catch (cause: unknown) {
            reload();
            toast.error(`Promotion updated, but the coupon was not saved: ${errorMessage(cause)}`);
            return;
          }
        }
        setDetail((current) => (current?.id === updated.id ? updated : current));
        toast.success("Promotion updated");
        reload();
        return;
      }

      const created = await client.promotions.create({
        title: draft.title,
        promotion_type: draft.promotionType,
        value: draft.value,
        status: draft.status,
        is_automatic: draft.isAutomatic,
      });
      if (code.length > 0) {
        try {
          await client.coupons.create({ code, promotion_id: created.id });
        } catch (cause: unknown) {
          reload();
          toast.error(`Promotion created, but the coupon was not saved: ${errorMessage(cause)}`);
          return;
        }
      }
      setPage(1);
      toast.success("Promotion created");
      reload();
    },
    [mode, editing, editingCoupon, client, reload],
  );

  const toggleStatus = useCallback(
    async (promotion: Promotion) => {
      const next: PromotionStatus = promotion.status === "active" ? "disabled" : "active";

      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        setDemoPromotions((current) =>
          current.map((entry) =>
            entry.id === promotion.id ? { ...entry, status: next, updated_at: now } : entry,
          ),
        );
        setDetail((current) =>
          current?.id === promotion.id ? { ...current, status: next, updated_at: now } : current,
        );
        toast.success(
          next === "active" ? "Promotion activated in the demo" : "Promotion disabled in the demo",
        );
        return;
      }

      if (!client) {
        toast.error("No store is selected.");
        return;
      }

      setActionBusyId(promotion.id);
      try {
        const updated = await client.promotions.update(promotion.id, { status: next });
        setDetail((current) => (current?.id === updated.id ? updated : current));
        toast.success(next === "active" ? "Promotion activated" : "Promotion disabled");
        reload();
      } catch (cause) {
        toast.error(errorMessage(cause));
      } finally {
        setActionBusyId(null);
      }
    },
    [mode, client, reload],
  );

  const confirmDelete = useCallback(async () => {
    if (!deleteTarget || deleteBusy) return;
    const target = deleteTarget;
    setDeleteBusy(true);
    try {
      if (mode === "demo") {
        setDemoPromotions((current) => current.filter((promotion) => promotion.id !== target.id));
        setDemoCoupons((current) => {
          const next = { ...current };
          delete next[target.id];
          return next;
        });
      } else {
        if (!client) {
          toast.error("No store is selected.");
          return;
        }
        await client.promotions.delete(target.id);
        reload();
      }
      if (detail?.id === target.id) setDetail(null);
      toast.success("Promotion deleted");
      setDeleteTarget(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDeleteBusy(false);
    }
  }, [deleteTarget, deleteBusy, mode, client, reload, detail?.id]);

  const emptyState = useMemo(() => {
    if (statusFilter === "all") {
      return {
        title: "No promotions yet",
        description:
          "Create an automatic discount or a coupon code to start driving repeat orders.",
        action: (
          <Button onClick={openCreate}>
            <Plus className="size-4" aria-hidden="true" />
            New promotion
          </Button>
        ),
      };
    }
    return {
      title: "No promotions with this status",
      description: `There are no ${PROMOTION_STATUS_LABELS[statusFilter].toLowerCase()} promotions right now.`,
      action: (
        <Button
          variant="outline"
          onClick={() => {
            setStatusFilter("all");
            setPage(1);
          }}
        >
          Show all promotions
        </Button>
      ),
    };
  }, [openCreate, statusFilter]);

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Promotions"
        description="Build automatic discounts and coupon codes, then track how often each offer is redeemed."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              disabled={loading}
              aria-label="Refresh promotions"
              title="Refresh promotions"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            <Button onClick={openCreate}>
              <Plus className="size-4" aria-hidden="true" />
              New promotion
            </Button>
          </>
        }
      />

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div
          role="group"
          aria-label="Filter promotions by status"
          className="flex min-w-0 items-center gap-1 overflow-x-auto rounded-full border border-border bg-secondary p-1 no-scrollbar"
        >
          {STATUS_FILTERS.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setStatusFilter(option.value);
                setPage(1);
              }}
              aria-pressed={statusFilter === option.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center rounded-full px-3 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                statusFilter === option.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
        <p className="text-[11.5px] text-muted-foreground">
          {total.toLocaleString("en-US")} {total === 1 ? "promotion" : "promotions"}
          {mode === "demo" ? " · sample data" : ""}
        </p>
      </div>

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">
              {statusFilter === "all"
                ? "All promotions"
                : `${PROMOTION_STATUS_LABELS[statusFilter]} promotions`}
            </p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Automatic discounts apply at checkout; coupon codes are redeemed by customers.
            </p>
          </div>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Promotion</TableHead>
              <TableHead className="hidden w-[130px] md:table-cell">Type</TableHead>
              <TableHead className="w-[130px]">Value</TableHead>
              <TableHead className="w-[110px]">Status</TableHead>
              <TableHead className="w-[150px]">Channel</TableHead>
              <TableHead className="w-[110px]">Usage</TableHead>
              <TableHead className="hidden w-[120px] lg:table-cell">Created</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((promotion) => (
              <TableRow key={promotion.id}>
                <TableCell className="pl-6">
                  <button
                    type="button"
                    onClick={() => openDetail(promotion)}
                    className="cursor-pointer text-left text-[13px] font-medium text-foreground underline-offset-4 hover:underline"
                  >
                    {promotion.title}
                  </button>
                  {promotion.code ? (
                    <p className="mt-1 font-mono text-[10.5px] text-faint">{promotion.code}</p>
                  ) : null}
                </TableCell>
                <TableCell className="hidden text-muted-foreground md:table-cell">
                  {PROMOTION_TYPE_LABELS[promotion.promotion_type]}
                </TableCell>
                <TableCell className="font-medium text-foreground tnum">
                  {formatPromotionValue(promotion.promotion_type, promotion.value, currency)}
                </TableCell>
                <TableCell>
                  <PromotionStatusBadge status={promotion.status} />
                </TableCell>
                <TableCell>
                  <Badge variant={promotion.is_automatic ? "info" : "secondary"}>
                    {promotion.is_automatic ? (
                      <Sparkles aria-hidden="true" />
                    ) : (
                      <TicketPercent aria-hidden="true" />
                    )}
                    <span className="max-w-[112px] truncate">
                      {promotionChannelLabel(promotion)}
                    </span>
                  </Badge>
                </TableCell>
                <TableCell className="text-[12px] text-ink-soft tnum">
                  {promotionUsageLabel(promotion.usage_count, promotion.usage_limit)}
                </TableCell>
                <TableCell className="hidden text-muted-foreground lg:table-cell">
                  {shortDate(fromUnixSeconds(promotion.created_at))}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Actions for ${promotion.title}`}
                      >
                        <MoreHorizontal className="size-4" aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-[190px]">
                      <DropdownMenuLabel className="truncate">{promotion.title}</DropdownMenuLabel>
                      <DropdownMenuItem onSelect={() => openDetail(promotion)}>
                        <Eye aria-hidden="true" />
                        View details
                      </DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => openEdit(promotion)}>
                        <Pencil aria-hidden="true" />
                        Edit
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={actionBusyId === promotion.id}
                        onSelect={() => void toggleStatus(promotion)}
                      >
                        {actionBusyId === promotion.id ? (
                          <Loader2 className="animate-spin" aria-hidden="true" />
                        ) : promotion.status === "active" ? (
                          <PowerOff aria-hidden="true" />
                        ) : (
                          <Power aria-hidden="true" />
                        )}
                        {promotion.status === "active" ? "Disable" : "Activate"}
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        variant="destructive"
                        onSelect={() => setDeleteTarget(promotion)}
                      >
                        <Trash2 aria-hidden="true" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
            {rows.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={8} className="p-0">
                  <EmptyState
                    title={emptyState.title}
                    description={emptyState.description}
                    action={emptyState.action}
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>

        <div className="flex flex-col gap-3 border-t border-border px-6 py-3.5 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[11.5px] text-muted-foreground">
            {total === 0
              ? "No promotions to show"
              : `Showing ${start}–${end} of ${total.toLocaleString("en-US")}`}
          </p>
          <Pagination page={currentPage} pages={pages} onChange={setPage} />
        </div>
      </Card>

      <PromotionDialog
        open={dialogOpen}
        onOpenChange={handleDialogOpenChange}
        promotion={editing}
        coupon={editingCoupon}
        couponLoading={couponLoading}
        currency={currency}
        onSubmit={handleSave}
      />

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deleteBusy) setDeleteTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete promotion?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.title}” and any coupon attached to it will be removed. This cannot be
              undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteTarget(null)} disabled={deleteBusy}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void confirmDelete()}
              disabled={deleteBusy}
            >
              {deleteBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Delete promotion
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Sheet
        open={detail !== null}
        onOpenChange={(open) => {
          if (!open) setDetail(null);
        }}
      >
        <SheetContent className="flex flex-col sm:max-w-lg">
          {detail ? (
            <>
              <SheetHeader>
                <div className="flex flex-wrap items-center gap-2 pr-8">
                  <PromotionStatusBadge status={detail.status} />
                  <Badge variant="outline">{PROMOTION_TYPE_LABELS[detail.promotion_type]}</Badge>
                  <Badge variant={detail.is_automatic ? "info" : "secondary"}>
                    {detail.is_automatic ? (
                      <Sparkles aria-hidden="true" />
                    ) : (
                      <TicketPercent aria-hidden="true" />
                    )}
                    {promotionChannelLabel(detail)}
                  </Badge>
                </div>
                <SheetTitle>{detail.title}</SheetTitle>
                <SheetDescription>
                  {formatPromotionValue(detail.promotion_type, detail.value, currency)} ·{" "}
                  {promotionUsageLabel(detail.usage_count, detail.usage_limit)} recorded
                </SheetDescription>
              </SheetHeader>

              <div className="flex-1 space-y-6 overflow-y-auto p-6">
                <section className="rounded-xl border border-border bg-secondary/40 p-4">
                  <p className="text-[11px] font-medium text-muted-foreground">Discount</p>
                  <p className="mt-1.5 font-display text-[32px] leading-none text-foreground">
                    {formatPromotionValue(detail.promotion_type, detail.value, currency)}
                  </p>
                  <p className="mt-2 text-[12px] text-muted-foreground">
                    {PROMOTION_TYPE_LABELS[detail.promotion_type]}
                    {detail.is_automatic
                      ? " · applied automatically at checkout"
                      : " · requires a coupon or promotion code"}
                  </p>
                </section>

                <section className="grid grid-cols-2 gap-4">
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Usage</p>
                    <p className="mt-1 text-[14px] font-medium text-foreground tnum">
                      {detailUsageLoading
                        ? "Loading…"
                        : promotionUsageLabel(
                            detailUsage ?? detail.usage_count,
                            detail.usage_limit,
                          )}
                    </p>
                    {detail.usage_limit !== null ? (
                      <ProgressBar
                        className="mt-2"
                        value={detailUsage ?? detail.usage_count}
                        max={detail.usage_limit}
                        tone={
                          (detailUsage ?? detail.usage_count) >= detail.usage_limit
                            ? "destructive"
                            : "brand"
                        }
                      />
                    ) : null}
                    {detailUsageError ? (
                      <p className="mt-1.5 text-[10.5px] text-warning">
                        Live usage unavailable — showing the recorded count.
                      </p>
                    ) : null}
                  </div>
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Created</p>
                    <p className="mt-1 text-[14px] font-medium text-foreground">
                      {shortDate(fromUnixSeconds(detail.created_at))}
                    </p>
                  </div>
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Starts</p>
                    <p className="mt-1 text-[12.5px] text-ink-soft">
                      {detail.starts_at === null
                        ? "Immediately"
                        : shortDate(fromUnixSeconds(detail.starts_at))}
                    </p>
                  </div>
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Ends</p>
                    <p className="mt-1 text-[12.5px] text-ink-soft">
                      {detail.ends_at === null
                        ? "No end date"
                        : shortDate(fromUnixSeconds(detail.ends_at))}
                    </p>
                  </div>
                </section>

                <section>
                  <p className="text-[11px] font-medium text-muted-foreground">Coupon</p>
                  {detailCouponLoading ? (
                    <p className="mt-2 text-[12px] text-muted-foreground">Loading coupon…</p>
                  ) : detailCouponError ? (
                    <p className="mt-2 text-[12px] text-warning">
                      Coupon details could not be loaded.
                    </p>
                  ) : detailCoupon ? (
                    <p className="mt-2 inline-flex items-center gap-2 rounded-lg border border-border bg-background px-3 py-2 font-mono text-[12.5px] text-foreground">
                      <TicketPercent className="size-3.5 text-faint" aria-hidden="true" />
                      {detailCoupon.code}
                    </p>
                  ) : (
                    <p className="mt-2 text-[12px] text-muted-foreground">
                      No coupon is attached to this promotion.
                    </p>
                  )}
                </section>

                <section>
                  <p className="text-[11px] font-medium text-muted-foreground">Rules</p>
                  {detail.rules.length > 0 ? (
                    <ul className="mt-2 space-y-2">
                      {detail.rules.map((rule) => (
                        <li
                          key={rule.id}
                          className="rounded-lg border border-border bg-background px-3 py-2 text-[12px] text-ink-soft"
                        >
                          {promotionRuleSummary(rule)}
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="mt-2 text-[12px] text-muted-foreground">
                      No rules — the promotion applies to every eligible cart.
                    </p>
                  )}
                </section>

                <p className="text-[10.5px] leading-relaxed text-faint">
                  Usage is reported as an aggregate count. The API does not expose individual
                  redemption records.
                </p>
              </div>

              <SheetFooter className="flex-wrap">
                <Button
                  variant="destructive"
                  onClick={() => {
                    const target = detail;
                    setDetail(null);
                    setDeleteTarget(target);
                  }}
                >
                  <Trash2 className="size-4" aria-hidden="true" />
                  Delete
                </Button>
                <Button
                  variant="outline"
                  onClick={() => {
                    const target = detail;
                    setDetail(null);
                    openEdit(target);
                  }}
                >
                  <Pencil className="size-4" aria-hidden="true" />
                  Edit
                </Button>
                <Button
                  onClick={() => void toggleStatus(detail)}
                  disabled={actionBusyId === detail.id}
                >
                  {actionBusyId === detail.id ? (
                    <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                  ) : detail.status === "active" ? (
                    <PowerOff className="size-4" aria-hidden="true" />
                  ) : (
                    <Power className="size-4" aria-hidden="true" />
                  )}
                  {detail.status === "active" ? "Disable" : "Activate"}
                </Button>
              </SheetFooter>
            </>
          ) : null}
        </SheetContent>
      </Sheet>
    </div>
  );
}

function PromotionStatusBadge({ status }: { status: PromotionStatus }) {
  const tone = PROMOTION_STATUS_TONES[status];
  return (
    <Badge variant={tone}>
      <span className={cn("size-1.5 rounded-full", STATUS_DOTS[tone])} aria-hidden="true" />
      {PROMOTION_STATUS_LABELS[status]}
    </Badge>
  );
}

function SelectField({
  id,
  label,
  value,
  onChange,
  disabled,
  children,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <div className="relative">
        <select
          id={id}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          disabled={disabled}
          className={selectClass}
        >
          {children}
        </select>
        <ChevronDown
          className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
          aria-hidden="true"
        />
      </div>
    </div>
  );
}

function PromotionDialog({
  open,
  onOpenChange,
  promotion,
  coupon,
  couponLoading,
  currency,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  promotion: Promotion | null;
  coupon: CouponRef | null;
  couponLoading: boolean;
  currency: string;
  onSubmit: (draft: PromotionDraft) => Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [type, setType] = useState<PromotionType>("percentage");
  const [valueInput, setValueInput] = useState("");
  const [status, setStatus] = useState<PromotionStatus>("draft");
  const [automatic, setAutomatic] = useState(false);
  const [couponCode, setCouponCode] = useState("");
  const [titleError, setTitleError] = useState<string | null>(null);
  const [valueError, setValueError] = useState<string | null>(null);
  const [couponError, setCouponError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const couponTouched = useRef(false);

  useEffect(() => {
    if (!open) return;
    setTitle(promotion?.title ?? "");
    setType(promotion?.promotion_type ?? "percentage");
    setValueInput(
      promotion ? promotionValueToInput(promotion.promotion_type, promotion.value) : "",
    );
    setStatus(promotion?.status ?? "draft");
    setAutomatic(promotion?.is_automatic ?? false);
    setCouponCode(coupon?.code ?? "");
    setTitleError(null);
    setValueError(null);
    setCouponError(null);
    setBusy(false);
    couponTouched.current = false;
  }, [open, promotion?.id]);

  useEffect(() => {
    if (!open || couponTouched.current) return;
    setCouponCode(coupon?.code ?? "");
  }, [open, coupon?.id, coupon?.code]);

  const changeType = (next: PromotionType) => {
    if (next === type) return;
    setType(next);
    setValueError(null);
    setValueInput(next === "free_shipping" ? "0" : "");
  };

  const submit = async () => {
    if (busy) return;
    const trimmedTitle = title.trim();
    if (trimmedTitle.length === 0) {
      setTitleError("Enter a name for this promotion.");
      return;
    }
    if (trimmedTitle.length > 120) {
      setTitleError("Promotion names cannot be longer than 120 characters.");
      return;
    }
    const parsed = parsePromotionValue(type, valueInput);
    if (!parsed.ok) {
      setValueError(parsed.error);
      return;
    }
    const couponProblem = couponCodeError(couponCode);
    if (couponProblem) {
      setCouponError(couponProblem);
      return;
    }

    setBusy(true);
    try {
      await onSubmit({
        title: trimmedTitle,
        promotionType: type,
        value: parsed.value,
        status,
        isAutomatic: automatic,
        couponCode: normalizeCouponCode(couponCode),
      });
      onOpenChange(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-xl">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{promotion ? "Edit promotion" : "New promotion"}</DialogTitle>
            <DialogDescription>
              {promotion
                ? "Update the discount or lifecycle state. Changes apply to the live store immediately."
                : "Set a discount, choose when it applies, and optionally attach a coupon code."}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="promotion-title">Name</Label>
            <Input
              id="promotion-title"
              value={title}
              onChange={(event) => {
                setTitle(event.target.value);
                setTitleError(null);
              }}
              placeholder="Summer studio edit"
              autoComplete="off"
              disabled={busy}
              aria-invalid={titleError !== null}
            />
            {titleError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {titleError}
              </p>
            ) : null}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <SelectField
              id="promotion-type"
              label="Type"
              value={type}
              onChange={(value) => changeType(value as PromotionType)}
              disabled={busy}
            >
              {PROMOTION_TYPES.map((option) => (
                <option key={option} value={option}>
                  {PROMOTION_TYPE_LABELS[option]}
                </option>
              ))}
            </SelectField>
            <SelectField
              id="promotion-status"
              label="Status"
              value={status}
              onChange={(value) => setStatus(value as PromotionStatus)}
              disabled={busy}
            >
              {PROMOTION_STATUSES.map((option) => (
                <option key={option} value={option}>
                  {PROMOTION_STATUS_LABELS[option]}
                </option>
              ))}
            </SelectField>
          </div>

          {type === "free_shipping" ? (
            <div className="rounded-lg border border-border bg-secondary/40 px-3.5 py-3 text-[12px] text-muted-foreground">
              Free shipping promotions do not have a discount amount.
            </div>
          ) : (
            <div className="space-y-2">
              <Label htmlFor="promotion-value">
                {type === "percentage" ? "Percentage" : `Amount (${currency})`}
              </Label>
              <div className="relative">
                <Input
                  id="promotion-value"
                  type="number"
                  min="0"
                  max={type === "percentage" ? "100" : undefined}
                  step="0.01"
                  inputMode="decimal"
                  value={valueInput}
                  onChange={(event) => {
                    setValueInput(event.target.value);
                    setValueError(null);
                  }}
                  placeholder={type === "percentage" ? "15" : "12.50"}
                  className={type === "percentage" ? "pr-8" : undefined}
                  disabled={busy}
                  aria-invalid={valueError !== null}
                />
                {type === "percentage" ? (
                  <span className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-[12px] text-faint">
                    %
                  </span>
                ) : null}
              </div>
              {valueError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {valueError}
                </p>
              ) : null}
            </div>
          )}

          <div className="flex items-start justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3.5">
            <div>
              <Label htmlFor="promotion-automatic">Automatic promotion</Label>
              <p className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">
                Applies at checkout without a coupon code.
              </p>
            </div>
            <Switch
              id="promotion-automatic"
              checked={automatic}
              onCheckedChange={setAutomatic}
              disabled={busy}
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="promotion-coupon">Coupon code</Label>
            <Input
              id="promotion-coupon"
              value={couponCode}
              onChange={(event) => {
                couponTouched.current = true;
                setCouponCode(event.target.value);
                setCouponError(null);
              }}
              placeholder="SAVE10"
              autoCapitalize="characters"
              autoComplete="off"
              spellCheck={false}
              disabled={busy || couponLoading}
              aria-invalid={couponError !== null}
              className="font-mono uppercase"
            />
            <p className="text-[11px] leading-relaxed text-faint">
              {couponLoading
                ? "Loading the existing coupon…"
                : promotion
                  ? "Set a code to add or rename a coupon for this promotion. Clearing the field leaves an existing coupon unchanged."
                  : "Optional. Creates a coupon tied to this promotion once it is saved."}
            </p>
            {couponError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {couponError}
              </p>
            ) : null}
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
              disabled={busy}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {promotion ? "Save changes" : "Create promotion"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
