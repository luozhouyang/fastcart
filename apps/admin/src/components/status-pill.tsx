import type {
  InquiryFormStatus,
  InquiryStatus,
  NotificationDeliveryStatus,
  PageStatus,
} from "@fastcart/shared-types";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { FulfillmentStatus, OrderStatus, PaymentStatus } from "@/data/mock";

type Tone = "brand" | "warning" | "info" | "destructive" | "secondary";

const DOT: Record<Tone, string> = {
  brand: "bg-brand",
  warning: "bg-warning",
  info: "bg-info",
  destructive: "bg-destructive",
  secondary: "bg-faint",
};

function Pill({ tone, label }: { tone: Tone; label: string }) {
  return (
    <Badge variant={tone}>
      <span className={cn("size-1.5 rounded-full", DOT[tone])} aria-hidden="true" />
      {label}
    </Badge>
  );
}

const ORDER: Record<OrderStatus, { label: string; tone: Tone }> = {
  pending: { label: "Pending", tone: "secondary" },
  confirmed: { label: "Confirmed", tone: "info" },
  processing: { label: "Processing", tone: "warning" },
  fulfilled: { label: "Fulfilled", tone: "brand" },
  completed: { label: "Completed", tone: "brand" },
  canceled: { label: "Canceled", tone: "destructive" },
};

const PAYMENT: Record<PaymentStatus, { label: string; tone: Tone }> = {
  awaiting: { label: "Awaiting", tone: "secondary" },
  authorized: { label: "Authorized", tone: "info" },
  captured: { label: "Paid", tone: "brand" },
  partially_refunded: { label: "Partial refund", tone: "warning" },
  refunded: { label: "Refunded", tone: "destructive" },
  failed: { label: "Failed", tone: "destructive" },
};

const FULFILLMENT: Record<FulfillmentStatus, { label: string; tone: Tone }> = {
  not_fulfilled: { label: "Unfulfilled", tone: "secondary" },
  partially_fulfilled: { label: "Partial", tone: "warning" },
  fulfilled: { label: "Fulfilled", tone: "brand" },
  returned: { label: "Returned", tone: "destructive" },
};

export function OrderPill({ status }: { status: OrderStatus }) {
  const entry = ORDER[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

export function PaymentPill({ status }: { status: PaymentStatus }) {
  const entry = PAYMENT[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

export function FulfillmentPill({ status }: { status: FulfillmentStatus }) {
  const entry = FULFILLMENT[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

export function ProductStatusPill({ status }: { status: "active" | "draft" | "archived" }) {
  const map = {
    active: { label: "Active", tone: "brand" as Tone },
    draft: { label: "Draft", tone: "secondary" as Tone },
    archived: { label: "Archived", tone: "info" as Tone },
  };
  const entry = map[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

export function CustomerStatusPill({
  status,
}: {
  status: "active" | "new" | "guest" | "disabled";
}) {
  const map = {
    active: { label: "Active", tone: "brand" as Tone },
    new: { label: "New", tone: "info" as Tone },
    guest: { label: "Guest", tone: "secondary" as Tone },
    disabled: { label: "Disabled", tone: "destructive" as Tone },
  };
  const entry = map[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

export function StockPill({ available }: { available: number }) {
  if (available <= 0) return <Pill tone="destructive" label="Out of stock" />;
  if (available <= 10) return <Pill tone="warning" label="Low stock" />;
  return <Pill tone="brand" label="In stock" />;
}

const FORM_STATUS: Record<InquiryFormStatus, { label: string; tone: Tone }> = {
  draft: { label: "Draft", tone: "secondary" },
  published: { label: "Published", tone: "brand" },
  archived: { label: "Archived", tone: "info" },
};

export function InquiryFormStatusPill({ status }: { status: InquiryFormStatus }) {
  const entry = FORM_STATUS[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

const PAGE_STATUS: Record<PageStatus, { label: string; tone: Tone }> = {
  draft: { label: "Draft", tone: "secondary" },
  published: { label: "Published", tone: "brand" },
  archived: { label: "Archived", tone: "info" },
};

export function PageStatusPill({ status }: { status: PageStatus }) {
  const entry = PAGE_STATUS[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

export function ThemeActivePill({ active }: { active: boolean }) {
  return <Pill tone={active ? "brand" : "secondary"} label={active ? "Active" : "Inactive"} />;
}

const INQUIRY_STATUS: Record<InquiryStatus, { label: string; tone: Tone }> = {
  new: { label: "New", tone: "info" },
  read: { label: "Read", tone: "secondary" },
  replied: { label: "Replied", tone: "brand" },
  closed: { label: "Closed", tone: "secondary" },
  spam: { label: "Spam", tone: "destructive" },
};

export function InquiryStatusPill({ status }: { status: InquiryStatus }) {
  const entry = INQUIRY_STATUS[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

const DELIVERY_STATUS: Record<NotificationDeliveryStatus, { label: string; tone: Tone }> = {
  pending: { label: "Pending", tone: "secondary" },
  queued: { label: "Queued", tone: "info" },
  sent: { label: "Sent", tone: "brand" },
  failed: { label: "Failed", tone: "destructive" },
  skipped: { label: "Skipped", tone: "warning" },
};

export function DeliveryStatusPill({ status }: { status: NotificationDeliveryStatus }) {
  const entry = DELIVERY_STATUS[status];
  return <Pill tone={entry.tone} label={entry.label} />;
}

export function StatusRail({ tone, className }: { tone: Tone; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("absolute top-3 bottom-3 left-0 w-[2px] rounded-full", DOT[tone], className)}
    />
  );
}

export const orderRailTone: Record<OrderStatus, Tone> = {
  pending: "secondary",
  confirmed: "info",
  processing: "warning",
  fulfilled: "brand",
  completed: "brand",
  canceled: "destructive",
};
