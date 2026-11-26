import { useCallback, useEffect, useState } from "react";
import { Ban, CircleAlert, CreditCard, Loader2, Plus, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import type { PaymentRefund } from "@fastcart/shared-types";

import { PaymentRefundDialog } from "@/components/payment-refund-dialog";
import {
  PaymentSessionActionDialog,
  SESSION_ACTION_PAST,
  type SessionAction,
} from "@/components/payment-session-action-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
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
import { demoPaymentRefunds, demoPaymentSession } from "@/lib/demo-commerce";
import { fromUnixSeconds, shortDate, timeOfDay } from "@/lib/format";
import { PERMISSIONS } from "@/lib/permissions";
import {
  cancelPaymentSession,
  capturePaymentSession,
  canRefundPayment,
  createPaymentRefund,
  failPaymentSession,
  listPaymentRefunds,
  paymentSessionId,
  paymentStatusLabel,
  paymentStatusTone,
  refundableMinor,
  refundedMinor,
  type AdminPayment,
} from "@/lib/payments";
import { formatMinorAmount } from "@/lib/pricing";
import { cn } from "@/lib/utils";

export function PaymentDetailSheet({
  payment,
  onOpenChange,
}: {
  payment: AdminPayment | null;
  onOpenChange: (open: boolean) => void;
}) {
  const { mode, store, can } = useAuth();
  const [current, setCurrent] = useState<AdminPayment | null>(payment);
  const [refunds, setRefunds] = useState<PaymentRefund[]>([]);
  const [refundsLoading, setRefundsLoading] = useState(false);
  const [refundsError, setRefundsError] = useState<string | null>(null);
  const [refundOpen, setRefundOpen] = useState(false);
  const [action, setAction] = useState<{ kind: SessionAction; sessionId: string } | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [version, setVersion] = useState(0);

  const paymentId = payment?.id ?? null;
  const canWrite = can(PERMISSIONS.payments.write);

  useEffect(() => {
    setCurrent(payment);
    setRefundOpen(false);
    setAction(null);
  }, [payment]);

  useEffect(() => {
    if (paymentId === null) {
      setRefunds([]);
      return;
    }
    setRefundsError(null);
    if (mode === "demo") {
      setRefunds(demoPaymentRefunds(paymentId));
      setRefundsLoading(false);
      return;
    }
    if (!store) return;
    const controller = new AbortController();
    setRefundsLoading(true);
    void listPaymentRefunds(store.id, paymentId, controller.signal)
      .then((rows) => {
        if (!controller.signal.aborted) setRefunds(rows);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setRefundsError(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setRefundsLoading(false);
      });
    return () => controller.abort();
  }, [paymentId, mode, store, version]);

  const refresh = useCallback(() => setVersion((value) => value + 1), []);

  const sessionId = current ? paymentSessionId(current) : null;
  const refundable = current ? refundableMinor(current, refunds) : 0;
  const refundableAllowed = current ? canRefundPayment(current, refunds) : false;

  const submitRefund = useCallback(
    async (amount: number, reason: string): Promise<void> => {
      if (!current) return;
      if (mode === "demo") {
        const timestamp = Math.floor(Date.now() / 1000);
        const created = {
          id: `refund_demo_${timestamp}`,
          payment_id: current.id,
          order_id: current.order_id,
          amount,
          currency_code: current.currency_code,
          status: "succeeded" as const,
          provider_refund_id: null,
          reason: reason.length === 0 ? null : reason,
          created_at: timestamp,
          updated_at: timestamp,
        };
        setRefunds((rows) => [created, ...rows]);
        setCurrent((row) =>
          row === null
            ? row
            : {
                ...row,
                status:
                  refundedMinor([...refunds, created]) >= row.amount
                    ? "refunded"
                    : "partially_refunded",
              },
        );
        toast.success("Refund created in the demo");
        return;
      }
      if (!store) throw new Error("No store is selected.");
      await createPaymentRefund(store.id, current.id, {
        amount,
        reason: reason.length === 0 ? null : reason,
      });
      toast.success("Refund created");
      refresh();
    },
    [current, mode, store, refunds, refresh],
  );

  const submitSessionAction = useCallback(async () => {
    if (!action || actionBusy) return;
    setActionBusy(true);
    try {
      if (mode === "demo") {
        const session = demoPaymentSession(action.sessionId);
        // Capture creates a payment for the session; cancel and fail only
        // change the session, so the recorded payment row stays as it is.
        if (action.kind === "capture") {
          setCurrent((row) =>
            row === null
              ? row
              : {
                  ...row,
                  amount: session.amount,
                  status: "captured",
                  captured_at: Math.floor(Date.now() / 1000),
                  metadata: { ...row.metadata, session_id: action.sessionId },
                },
          );
        }
        toast.success(`Session ${SESSION_ACTION_PAST[action.kind]} in the demo`);
      } else {
        if (!store) throw new Error("No store is selected.");
        if (action.kind === "capture") {
          const captured = await capturePaymentSession(store.id, action.sessionId);
          setCurrent(captured);
          toast.success("Payment session captured");
        } else if (action.kind === "cancel") {
          await cancelPaymentSession(store.id, action.sessionId);
          toast.success("Payment session canceled");
        } else {
          await failPaymentSession(store.id, action.sessionId, {
            reason: "failed from the admin",
          });
          toast.success("Payment session failed");
        }
        refresh();
      }
      setAction(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setActionBusy(false);
    }
  }, [action, actionBusy, mode, store, current?.id, refresh]);

  return (
    <>
      <Sheet
        open={payment !== null}
        onOpenChange={(open) => {
          if (!open) {
            setCurrent(null);
            setRefunds([]);
          }
          onOpenChange(open);
        }}
      >
        <SheetContent className="flex flex-col sm:max-w-2xl">
          {current ? (
            <>
              <SheetHeader>
                <div className="flex flex-wrap items-center gap-2 pr-8">
                  <Badge variant={paymentStatusTone(current.status)}>
                    {paymentStatusLabel(current.status)}
                  </Badge>
                  <Badge variant="outline">{current.provider}</Badge>
                  {current.order_id ? (
                    <span className="font-mono text-[10.5px] text-faint">{current.order_id}</span>
                  ) : null}
                </div>
                <SheetTitle>{formatMinorAmount(current.amount, current.currency_code)}</SheetTitle>
                <SheetDescription>
                  {current.captured_at === null
                    ? "Not captured"
                    : `Captured ${shortDate(fromUnixSeconds(current.captured_at))} at ${timeOfDay(
                        fromUnixSeconds(current.captured_at),
                      )}`}
                </SheetDescription>
              </SheetHeader>

              <div className="flex-1 space-y-6 overflow-y-auto p-6">
                <section className="grid grid-cols-2 gap-4">
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Payment</p>
                    <p className="mt-1 font-mono text-[11.5px] break-all text-ink-soft">
                      {current.id}
                    </p>
                  </div>
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Provider id</p>
                    <p className="mt-1 font-mono text-[11.5px] break-all text-ink-soft">
                      {current.provider_payment_id ?? "—"}
                    </p>
                  </div>
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Order</p>
                    <p className="mt-1 font-mono text-[11.5px] break-all text-ink-soft">
                      {current.order_id ?? "Not linked"}
                    </p>
                  </div>
                  <div>
                    <p className="text-[11px] font-medium text-muted-foreground">Refunded</p>
                    <p className="mt-1 text-[12.5px] text-ink-soft tnum">
                      {formatMinorAmount(refundedMinor(refunds), current.currency_code)}
                      {refundable > 0
                        ? ` · ${formatMinorAmount(refundable, current.currency_code)} refundable`
                        : ""}
                    </p>
                  </div>
                </section>

                {sessionId !== null ? (
                  <section className="rounded-xl border border-border bg-secondary/40 p-4">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div>
                        <p className="text-[12px] font-medium text-foreground">Payment session</p>
                        <p className="mt-1 font-mono text-[11px] break-all text-faint">
                          {sessionId}
                        </p>
                      </div>
                      {canWrite ? (
                        <div className="flex flex-wrap gap-1.5">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => setAction({ kind: "capture", sessionId })}
                          >
                            <CreditCard className="size-3.5" aria-hidden="true" />
                            Capture
                          </Button>
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => setAction({ kind: "cancel", sessionId })}
                          >
                            <Ban className="size-3.5" aria-hidden="true" />
                            Cancel
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="text-destructive hover:text-destructive"
                            onClick={() => setAction({ kind: "fail", sessionId })}
                          >
                            <CircleAlert className="size-3.5" aria-hidden="true" />
                            Fail
                          </Button>
                        </div>
                      ) : null}
                    </div>
                  </section>
                ) : null}

                <section>
                  <div className="flex items-center justify-between">
                    <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
                      Refunds
                    </p>
                    <div className="flex items-center gap-1.5">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={refresh}
                        disabled={refundsLoading}
                        aria-label="Refresh refunds"
                      >
                        <RefreshCw
                          className={cn("size-3.5", refundsLoading && "animate-spin")}
                          aria-hidden="true"
                        />
                      </Button>
                      {canWrite ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => setRefundOpen(true)}
                          disabled={!refundableAllowed}
                        >
                          <Plus className="size-3.5" aria-hidden="true" />
                          New refund
                        </Button>
                      ) : null}
                    </div>
                  </div>

                  {refundsError ? (
                    <p className="mt-2 text-[12px] text-destructive">{refundsError}</p>
                  ) : refundsLoading ? (
                    <div className="flex justify-center py-8">
                      <Loader2 className="size-4 animate-spin text-faint" aria-hidden="true" />
                    </div>
                  ) : refunds.length === 0 ? (
                    <p className="mt-2 rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12px] text-muted-foreground">
                      No refunds recorded for this payment.
                    </p>
                  ) : (
                    <div className="mt-2 overflow-hidden rounded-lg border border-border">
                      <Table>
                        <TableHeader>
                          <TableRow className="hover:bg-transparent">
                            <TableHead>Amount</TableHead>
                            <TableHead className="w-[110px]">Status</TableHead>
                            <TableHead className="hidden md:table-cell">Reason</TableHead>
                            <TableHead className="w-[120px]">Created</TableHead>
                          </TableRow>
                        </TableHeader>
                        <TableBody>
                          {refunds.map((refund) => (
                            <TableRow key={refund.id}>
                              <TableCell className="tnum">
                                {formatMinorAmount(refund.amount, refund.currency_code)}
                              </TableCell>
                              <TableCell>
                                <Badge
                                  variant={
                                    refund.status === "succeeded"
                                      ? "brand"
                                      : refund.status === "failed"
                                        ? "destructive"
                                        : "warning"
                                  }
                                >
                                  {refund.status}
                                </Badge>
                              </TableCell>
                              <TableCell className="hidden max-w-[200px] truncate text-[12px] text-muted-foreground md:table-cell">
                                {refund.reason ?? "—"}
                              </TableCell>
                              <TableCell className="text-[12px] text-muted-foreground">
                                {shortDate(fromUnixSeconds(refund.created_at))}
                              </TableCell>
                            </TableRow>
                          ))}
                        </TableBody>
                      </Table>
                    </div>
                  )}
                </section>
              </div>

              <SheetFooter>
                <Button variant="ghost" onClick={() => onOpenChange(false)}>
                  Close
                </Button>
              </SheetFooter>
            </>
          ) : null}
        </SheetContent>
      </Sheet>

      <PaymentRefundDialog
        open={refundOpen}
        maxMinor={refundable}
        currency={current?.currency_code ?? "USD"}
        onOpenChange={setRefundOpen}
        onSubmit={submitRefund}
      />

      <PaymentSessionActionDialog
        action={action?.kind ?? null}
        busy={actionBusy}
        onCancel={() => setAction(null)}
        onConfirm={() => void submitSessionAction()}
      />
    </>
  );
}
