import { useEffect, useState, type ReactNode } from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { Loader2, X } from "lucide-react";
import { toast } from "sonner";

import { PaymentPill, OrderPill, FulfillmentPill } from "@/components/status-pill";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { errorMessage } from "@/lib/api";
import { initials, money, monthDay, timeOfDay } from "@/lib/format";
import type { OrderView } from "@/lib/models";
import { cn } from "@/lib/utils";

type Action = "fulfill" | "refund";

const dotTone: Record<OrderView["history"][number]["tone"], string> = {
  brand: "bg-brand",
  warning: "bg-warning",
  muted: "bg-faint",
};

function DetailLabel({ children }: { children: ReactNode }) {
  return <p className="text-[11px] font-medium text-muted-foreground">{children}</p>;
}

export function OrderDrawer({
  order,
  onClose,
  onFulfill,
  onRefund,
}: {
  order: OrderView | null;
  onClose: () => void;
  onFulfill: (order: OrderView, tracking: string) => Promise<void>;
  onRefund: (order: OrderView, amount: number, reason: string) => Promise<void>;
}) {
  const [action, setAction] = useState<Action | null>(null);
  const [tracking, setTracking] = useState("");
  const [amount, setAmount] = useState("");
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);

  const remaining = order ? Math.max(0, order.total - order.refundedTotal) : 0;

  useEffect(() => {
    setAction(null);
    setTracking("");
    setReason("");
    setAmount(remaining > 0 ? remaining.toFixed(2) : "");
  }, [order?.id, remaining]);

  const submitFulfill = async () => {
    if (!order || busy) return;
    setBusy(true);
    try {
      await onFulfill(order, tracking);
      setAction(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const submitRefund = async () => {
    if (!order || busy) return;
    const parsed = Number(amount);
    if (!Number.isFinite(parsed) || parsed <= 0) {
      toast.error("Enter a refund amount greater than zero.");
      return;
    }
    if (parsed > remaining + 0.001) {
      toast.error(`Refunds cannot exceed ${money(remaining, order.currency)}.`);
      return;
    }
    setBusy(true);
    try {
      await onRefund(order, parsed, reason);
      setAction(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <DialogPrimitive.Root open={order !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/30 backdrop-blur-[2px] data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0" />
        {order ? (
          <DialogPrimitive.Content
            className={cn(
              "fixed top-2 right-2 bottom-2 z-50 flex w-[min(480px,calc(100vw-1rem))] flex-col overflow-hidden rounded-2xl border border-border bg-card text-card-foreground shadow-lift outline-none",
              "sm:top-4 sm:right-4 sm:bottom-4 sm:w-[480px]",
              "data-[state=closed]:animate-out data-[state=closed]:duration-200 data-[state=closed]:slide-out-to-right",
              "data-[state=open]:animate-in data-[state=open]:duration-300 data-[state=open]:slide-in-from-right",
            )}
          >
            <DialogPrimitive.Title className="sr-only">Order #{order.number}</DialogPrimitive.Title>
            <DialogPrimitive.Description className="sr-only">
              Order details, timeline and fulfillment actions
            </DialogPrimitive.Description>

            <header className="flex items-center justify-between gap-4 border-b border-border px-6 py-4">
              <div className="flex items-center gap-2.5">
                <span className="font-mono text-[12px] text-muted-foreground">#{order.number}</span>
                <OrderPill status={order.status} />
              </div>
              <DialogPrimitive.Close asChild>
                <button
                  type="button"
                  aria-label="Close order"
                  className="flex size-8 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/30 focus-visible:outline-none"
                >
                  <X className="size-4" />
                </button>
              </DialogPrimitive.Close>
            </header>

            <div className="flex-1 space-y-6 overflow-y-auto px-6 py-5">
              <section className="flex items-start gap-3.5">
                <Avatar className="size-10">
                  <AvatarFallback>{initials(order.customer)}</AvatarFallback>
                </Avatar>
                <div className="min-w-0 flex-1">
                  <p className="text-[14px] leading-tight font-medium text-foreground">
                    {order.customer}
                  </p>
                  <p className="mt-1 truncate text-[12px] text-muted-foreground">{order.email}</p>
                  <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground">
                    {order.address}
                  </p>
                </div>
              </section>

              <section>
                <DetailLabel>Items</DetailLabel>
                <ul className="mt-3 divide-y divide-border/70">
                  {order.items.map((item, index) => (
                    <li
                      key={`${item.sku}-${index}`}
                      className="flex items-start gap-3 py-3 first:pt-0"
                    >
                      <span className="mt-0.5 flex h-6 min-w-6 items-center justify-center rounded-[7px] border border-border bg-secondary px-1 font-mono text-[10.5px] text-ink-soft">
                        {item.quantity}×
                      </span>
                      <div className="min-w-0 flex-1">
                        <p className="text-[12.5px] leading-tight font-medium text-foreground">
                          {item.title}
                        </p>
                        <p className="mt-1 font-mono text-[10.5px] text-faint">
                          {item.variant} · {item.sku}
                        </p>
                      </div>
                      <span className="text-[12.5px] text-foreground tnum">
                        {money(item.price * item.quantity, order.currency)}
                      </span>
                    </li>
                  ))}
                </ul>
              </section>

              <section className="rounded-xl bg-secondary/55 p-4">
                <dl className="space-y-1.5 text-[12px]">
                  <div className="flex justify-between gap-3">
                    <dt className="text-muted-foreground">Subtotal</dt>
                    <dd className="text-ink-soft tnum">
                      {money(order.total - order.shipping - order.tax, order.currency)}
                    </dd>
                  </div>
                  <div className="flex justify-between gap-3">
                    <dt className="text-muted-foreground">Shipping</dt>
                    <dd className="text-ink-soft tnum">
                      {order.shipping === 0 ? "Free" : money(order.shipping, order.currency)}
                    </dd>
                  </div>
                  <div className="flex justify-between gap-3">
                    <dt className="text-muted-foreground">Tax</dt>
                    <dd className="text-ink-soft tnum">{money(order.tax, order.currency)}</dd>
                  </div>
                  {order.refundedTotal > 0 ? (
                    <div className="flex justify-between gap-3">
                      <dt className="text-muted-foreground">Refunded</dt>
                      <dd className="text-destructive tnum">
                        −{money(order.refundedTotal, order.currency)}
                      </dd>
                    </div>
                  ) : null}
                </dl>
                <div className="mt-3 flex items-baseline justify-between border-t border-border pt-3">
                  <span className="text-[12px] text-muted-foreground">Total</span>
                  <span className="text-[19px] font-medium tracking-[-0.02em] text-foreground tnum">
                    {money(order.total, order.currency)}
                  </span>
                </div>
              </section>

              <section>
                <DetailLabel>Timeline</DetailLabel>
                <ol className="relative mt-3 space-y-3.5 pl-4">
                  <span
                    className="absolute top-1.5 bottom-1.5 left-[3px] w-px bg-border"
                    aria-hidden="true"
                  />
                  {order.history.map((entry, index) => (
                    <li
                      key={`${entry.label}-${index}`}
                      className="relative flex items-start justify-between gap-3"
                    >
                      <span
                        className={cn(
                          "absolute top-1 -left-4 size-[7px] rounded-full ring-2 ring-card",
                          dotTone[entry.tone],
                        )}
                        aria-hidden="true"
                      />
                      <span className="text-[12px] text-ink-soft">{entry.label}</span>
                      <span className="font-mono text-[10px] whitespace-nowrap text-faint">
                        {monthDay(entry.at)} · {timeOfDay(entry.at)}
                      </span>
                    </li>
                  ))}
                </ol>
              </section>

              <section className="grid gap-4 sm:grid-cols-2">
                <div>
                  <DetailLabel>Payment</DetailLabel>
                  <div className="mt-2">
                    <PaymentPill status={order.paymentStatus} />
                  </div>
                  <p className="mt-2 text-[11px] text-muted-foreground">
                    {order.paymentMethod ??
                      (order.paymentStatus === "awaiting"
                        ? "Not charged yet"
                        : "Recorded at checkout")}
                  </p>
                </div>
                <div>
                  <DetailLabel>Fulfillment</DetailLabel>
                  <div className="mt-2">
                    <FulfillmentPill status={order.fulfillmentStatus} />
                  </div>
                  <p className="mt-2 text-[11px] text-muted-foreground">
                    {order.shippingMethod ?? "Shipping method not recorded"}
                  </p>
                </div>
              </section>

              {order.note ? (
                <section>
                  <DetailLabel>Customer note</DetailLabel>
                  <p className="mt-2 rounded-xl border border-border bg-background px-3.5 py-3 text-[12px] leading-relaxed text-ink-soft">
                    {order.note}
                  </p>
                </section>
              ) : null}
            </div>

            <footer className="flex gap-2.5 border-t border-border bg-card px-6 py-4">
              <Button
                variant="outline"
                className="flex-1"
                disabled={!order.canRefund}
                onClick={() => setAction("refund")}
              >
                Refund
              </Button>
              <Button
                className="flex-1"
                disabled={!order.canFulfill}
                onClick={() => setAction("fulfill")}
              >
                {order.canFulfill ? "Fulfill order" : "Fulfilled"}
              </Button>
            </footer>
          </DialogPrimitive.Content>
        ) : null}
      </DialogPrimitive.Portal>

      <Dialog open={action === "fulfill"} onOpenChange={(open) => !open && setAction(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Fulfill order #{order?.number}</DialogTitle>
            <DialogDescription>
              Mark every item in this order as shipped. A tracking number is optional.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="tracking">Tracking number</Label>
            <Input
              id="tracking"
              placeholder="1Z999AA10123456784"
              value={tracking}
              onChange={(event) => setTracking(event.target.value)}
              disabled={busy}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setAction(null)} disabled={busy}>
              Cancel
            </Button>
            <Button onClick={() => void submitFulfill()} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              Mark as fulfilled
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={action === "refund"} onOpenChange={(open) => !open && setAction(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Refund order #{order?.number}</DialogTitle>
            <DialogDescription>
              The refund is sent to the original payment method through the payment provider.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="refund-amount">
                Amount{" "}
                <span className="font-normal text-faint">
                  up to {order ? money(remaining, order.currency) : "—"}
                </span>
              </Label>
              <Input
                id="refund-amount"
                type="number"
                min="0.01"
                step="0.01"
                value={amount}
                onChange={(event) => setAmount(event.target.value)}
                disabled={busy}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="refund-reason">Reason</Label>
              <Input
                id="refund-reason"
                placeholder="Returned damaged"
                value={reason}
                onChange={(event) => setReason(event.target.value)}
                disabled={busy}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setAction(null)} disabled={busy}>
              Cancel
            </Button>
            <Button onClick={() => void submitRefund()} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              Refund payment
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </DialogPrimitive.Root>
  );
}
