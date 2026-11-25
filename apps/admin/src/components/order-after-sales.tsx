import type { AdminClient } from "@fastcart/admin-sdk";
import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
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
import { money } from "@/lib/format";
import { toMajor, type OrderView } from "@/lib/models";
import {
  buildClaimRequest,
  buildExchangeRequest,
  buildOrderEditRequest,
  buildReturnRequest,
  collectDifference,
  CREATION_HINTS,
  CREATION_LABELS,
  orderAfterSales,
  refundDifference,
  type CreationKind,
  type DifferenceKind,
  type OrderAfterSalesData,
} from "@/lib/order-after-sales";

type BadgeTone = "secondary" | "info" | "warning" | "brand" | "destructive";

const STATUS_META: Record<string, { label: string; tone: BadgeTone }> = {
  draft: { label: "Draft", tone: "secondary" },
  requested: { label: "Requested", tone: "info" },
  partially_received: { label: "Partially received", tone: "warning" },
  received: { label: "Received", tone: "brand" },
  written_off: { label: "Written off", tone: "warning" },
  confirmed: { label: "Confirmed", tone: "brand" },
  accepted: { label: "Accepted", tone: "brand" },
  canceled: { label: "Canceled", tone: "destructive" },
};

function AfterSaleStatus({ status }: { status: string }) {
  const meta = STATUS_META[status] ?? { label: status, tone: "secondary" as BadgeTone };
  return <Badge variant={meta.tone}>{meta.label}</Badge>;
}

function shortId(id: string): string {
  return id.length <= 8 ? id : id.slice(-8);
}

function unitCount(items: Array<{ quantity: number }>): string {
  const total = items.reduce((sum, item) => sum + item.quantity, 0);
  return `${total} ${total === 1 ? "unit" : "units"}`;
}

/**
 * Payment summary and every post-purchase record of one order.
 *
 * The panel owns its action state; the drawer owns loading and refetches the
 * data after every successful action through `onChanged`.
 */
export function OrderAfterSalesPanel({
  order,
  client,
  data,
  error,
  onRetry,
  onChanged,
  canWrite,
}: {
  order: OrderView;
  client: AdminClient;
  data: OrderAfterSalesData | null;
  error: string | null;
  onRetry: () => void;
  onChanged: () => void;
  canWrite: boolean;
}) {
  const [recordBusy, setRecordBusy] = useState<string | null>(null);
  const [creation, setCreation] = useState<CreationKind | null>(null);
  const [draftQuantities, setDraftQuantities] = useState<Record<string, number>>({});
  const [draftReason, setDraftReason] = useState("");
  const [replacementVariant, setReplacementVariant] = useState("");
  const [creating, setCreating] = useState(false);
  const [collectTarget, setCollectTarget] = useState<{ kind: DifferenceKind; id: string } | null>(
    null,
  );
  const [provider, setProvider] = useState("manual");

  useEffect(() => {
    setCreation(null);
    setDraftQuantities({});
    setDraftReason("");
    setReplacementVariant("");
    setCollectTarget(null);
  }, [order.id]);

  const runRecordAction = async (key: string, work: () => Promise<unknown>) => {
    if (recordBusy) return;
    setRecordBusy(key);
    try {
      await work();
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRecordBusy(null);
    }
  };

  const openCreation = (kind: CreationKind) => {
    setCreation(kind);
    setDraftReason("");
    setReplacementVariant("");
    setDraftQuantities(
      Object.fromEntries(
        order.items.map((item) => [item.id ?? item.sku, kind === "edit" ? item.quantity : 0]),
      ),
    );
  };

  const submitCreation = async () => {
    if (!creation || creating) return;
    setCreating(true);
    try {
      switch (creation) {
        case "edit":
          await client.orders.edits.create(
            order.id,
            buildOrderEditRequest(order.items, draftQuantities, draftReason),
          );
          break;
        case "return":
          await client.orders.returns.create(
            order.id,
            buildReturnRequest(order.items, draftQuantities, draftReason),
          );
          break;
        case "claim":
          await client.orders.claims.create(
            order.id,
            buildClaimRequest(order.items, draftQuantities, draftReason),
          );
          break;
        case "exchange":
          await client.orders.exchanges.create(
            order.id,
            buildExchangeRequest(order.items, draftQuantities, replacementVariant, draftReason),
          );
          break;
      }
      toast.success(`${CREATION_LABELS[creation]} saved.`);
      setCreation(null);
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCreating(false);
    }
  };

  const submitCollect = async () => {
    if (!collectTarget || recordBusy) return;
    const trimmed = provider.trim();
    if (trimmed.length === 0) {
      toast.error("Enter the payment provider.");
      return;
    }
    const target = collectTarget;
    setCollectTarget(null);
    await runRecordAction(`collect-${target.kind}-${target.id}`, () =>
      collectDifference(client, order.id, target.kind, target.id, trimmed),
    );
  };

  const differenceActions = (kind: DifferenceKind, recordId: string, difference: number | null) => {
    if (difference === null || difference === 0) return null;
    if (difference > 0) {
      const key = `collect-${kind}-${recordId}`;
      return (
        <Button
          size="sm"
          variant="outline"
          disabled={recordBusy !== null}
          onClick={() => {
            setProvider("manual");
            setCollectTarget({ kind, id: recordId });
          }}
        >
          Collect {money(toMajor(difference), order.currency)}
          {recordBusy === key ? (
            <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
          ) : null}
        </Button>
      );
    }
    const key = `refund-diff-${kind}-${recordId}`;
    return (
      <Button
        size="sm"
        variant="outline"
        disabled={recordBusy !== null}
        onClick={() =>
          void runRecordAction(key, () => refundDifference(client, order.id, kind, recordId))
        }
      >
        Refund {money(toMajor(-difference), order.currency)}
        {recordBusy === key ? (
          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
        ) : null}
      </Button>
    );
  };

  const differenceNote = (difference: number | null) =>
    difference !== null && difference !== 0 ? (
      <p className="mt-1 text-[11px] text-ink-soft tnum">
        Difference {money(toMajor(difference), order.currency)}
        {difference > 0 ? " due from the customer" : " due to the customer"}
      </p>
    ) : null;

  const paymentSummary = data?.paymentSummary;
  const records = data
    ? data.edits.length + data.returns.length + data.claims.length + data.exchanges.length
    : 0;

  return (
    <>
      {paymentSummary ? (
        <section className="rounded-xl border border-border bg-background p-4">
          <div className="flex items-center justify-between gap-3">
            <p className="text-[11px] font-medium text-muted-foreground">Payment summary</p>
            <span className="font-mono text-[10px] text-faint">
              {money(toMajor(paymentSummary.net_paid), order.currency)} net paid
            </span>
          </div>
          <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-2 text-[12px]">
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">Total</dt>
              <dd className="text-ink-soft tnum">
                {money(toMajor(paymentSummary.total), order.currency)}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">Captured</dt>
              <dd className="text-ink-soft tnum">
                {money(toMajor(paymentSummary.captured_total), order.currency)}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">Refunded</dt>
              <dd className="text-ink-soft tnum">
                {money(toMajor(paymentSummary.refunded_total), order.currency)}
              </dd>
            </div>
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">Outstanding</dt>
              <dd
                className={
                  paymentSummary.outstanding > 0
                    ? "text-destructive tnum"
                    : paymentSummary.outstanding < 0
                      ? "text-brand-ink tnum"
                      : "text-ink-soft tnum"
                }
              >
                {money(toMajor(paymentSummary.outstanding), order.currency)}
              </dd>
            </div>
          </dl>
          {paymentSummary.outstanding > 0 ? (
            <p className="mt-3 text-[11px] text-muted-foreground">
              The customer still owes {money(toMajor(paymentSummary.outstanding), order.currency)}.
            </p>
          ) : paymentSummary.refund_due > 0 ? (
            <p className="mt-3 text-[11px] text-muted-foreground">
              A refund of {money(toMajor(paymentSummary.refund_due), order.currency)} is due to the
              customer.
            </p>
          ) : null}
        </section>
      ) : null}

      <section>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-[11px] font-medium text-muted-foreground">After-sales</p>
          <div className="flex flex-wrap gap-1.5">
            {(["edit", "return", "claim", "exchange"] as CreationKind[]).map((kind) => (
              <Button
                key={kind}
                size="sm"
                variant="outline"
                disabled={!canWrite}
                onClick={() => openCreation(kind)}
              >
                {kind === "edit"
                  ? "Edit"
                  : kind === "return"
                    ? "Return"
                    : kind === "claim"
                      ? "Claim"
                      : "Exchange"}
              </Button>
            ))}
          </div>
        </div>

        {error ? (
          <div className="mt-3 rounded-xl border border-destructive/20 bg-destructive/5 px-3.5 py-3">
            <p className="text-[11.5px] leading-relaxed text-destructive">{error}</p>
            <Button size="sm" variant="ghost" className="mt-1.5" onClick={onRetry}>
              Retry
            </Button>
          </div>
        ) : !data ? (
          <p className="mt-2 text-[11.5px] text-muted-foreground">Loading after-sales records...</p>
        ) : records === 0 ? (
          <p className="mt-3 rounded-xl border border-border bg-background px-3.5 py-3 text-[11.5px] leading-relaxed text-muted-foreground">
            No edits, returns, claims or exchanges recorded for this order.
          </p>
        ) : (
          <div className="mt-3 space-y-4">
            {data.edits.length > 0 ? (
              <div>
                <p className="text-[11.5px] font-medium text-ink-soft">Edits</p>
                <ul className="mt-2 divide-y divide-border/70">
                  {data.edits.map((edit) => (
                    <li key={edit.id} className="py-3 first:pt-0 last:pb-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-mono text-[10px] text-faint">
                          #{shortId(edit.id)}
                        </span>
                        <AfterSaleStatus status={edit.status} />
                      </div>
                      <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground">
                        {edit.reason ?? "No reason recorded"} · {edit.items.length}{" "}
                        {edit.items.length === 1 ? "line" : "lines"} ·{" "}
                        {money(toMajor(edit.before_total), order.currency)} →{" "}
                        {money(toMajor(edit.after_total), order.currency)}
                      </p>
                      {differenceNote(edit.difference_total)}
                      <div className="mt-2 flex flex-wrap gap-1.5">
                        {edit.status === "draft" && canWrite ? (
                          <>
                            <Button
                              size="sm"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`confirm-edit-${edit.id}`, () =>
                                  orderAfterSales.confirmEdit(client, order.id, edit.id),
                                )
                              }
                            >
                              {recordBusy === `confirm-edit-${edit.id}` ? (
                                <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                              ) : null}
                              Confirm
                            </Button>
                            <Button
                              size="sm"
                              variant="ghost"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`cancel-edit-${edit.id}`, () =>
                                  orderAfterSales.cancelEdit(client, order.id, edit.id),
                                )
                              }
                            >
                              Cancel edit
                            </Button>
                          </>
                        ) : null}
                        {edit.status === "confirmed"
                          ? differenceActions("edit", edit.id, edit.difference_total)
                          : null}
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}

            {data.returns.length > 0 ? (
              <div>
                <p className="text-[11.5px] font-medium text-ink-soft">Returns</p>
                <ul className="mt-2 divide-y divide-border/70">
                  {data.returns.map((orderReturn) => (
                    <li key={orderReturn.id} className="py-3 first:pt-0 last:pb-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-mono text-[10px] text-faint">
                          #{shortId(orderReturn.id)}
                        </span>
                        <AfterSaleStatus status={orderReturn.status} />
                      </div>
                      <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground">
                        {orderReturn.reason ?? "No reason recorded"} ·{" "}
                        {unitCount(orderReturn.items)}
                      </p>
                      <div className="mt-2 flex flex-wrap gap-1.5">
                        {orderReturn.status === "draft" && canWrite ? (
                          <>
                            <Button
                              size="sm"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`request-return-${orderReturn.id}`, () =>
                                  orderAfterSales.requestReturn(client, order.id, orderReturn.id),
                                )
                              }
                            >
                              Request
                            </Button>
                            <Button
                              size="sm"
                              variant="ghost"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`cancel-return-${orderReturn.id}`, () =>
                                  orderAfterSales.cancelReturn(client, order.id, orderReturn.id),
                                )
                              }
                            >
                              Cancel
                            </Button>
                          </>
                        ) : null}
                        {orderReturn.status === "requested" ||
                        orderReturn.status === "partially_received" ? (
                          <>
                            <Button
                              size="sm"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`receive-return-${orderReturn.id}`, () =>
                                  orderAfterSales.receiveReturn(client, order.id, orderReturn.id),
                                )
                              }
                            >
                              Receive all
                            </Button>
                            <Button
                              size="sm"
                              variant="ghost"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`cancel-return-${orderReturn.id}`, () =>
                                  orderAfterSales.cancelReturn(client, order.id, orderReturn.id),
                                )
                              }
                            >
                              Cancel
                            </Button>
                          </>
                        ) : null}
                        {orderReturn.status === "received" ? (
                          <>
                            <Button
                              size="sm"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`refund-return-${orderReturn.id}`, () =>
                                  orderAfterSales.refundReturn(client, order.id, orderReturn.id),
                                )
                              }
                            >
                              Refund received
                            </Button>
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`write-off-return-${orderReturn.id}`, () =>
                                  orderAfterSales.writeOffReturn(client, order.id, orderReturn.id),
                                )
                              }
                            >
                              Write off
                            </Button>
                          </>
                        ) : null}
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}

            {data.claims.length > 0 ? (
              <div>
                <p className="text-[11.5px] font-medium text-ink-soft">Claims</p>
                <ul className="mt-2 divide-y divide-border/70">
                  {data.claims.map((claim) => (
                    <li key={claim.id} className="py-3 first:pt-0 last:pb-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-mono text-[10px] text-faint">
                          #{shortId(claim.id)}
                        </span>
                        <AfterSaleStatus status={claim.status} />
                      </div>
                      <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground">
                        {claim.reason ?? "No reason recorded"} · {unitCount(claim.items)}
                      </p>
                      {differenceNote(claim.difference_total)}
                      <div className="mt-2 flex flex-wrap gap-1.5">
                        {claim.status === "requested" && canWrite ? (
                          <>
                            <Button
                              size="sm"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`accept-claim-${claim.id}`, () =>
                                  orderAfterSales.acceptClaim(client, order.id, claim.id),
                                )
                              }
                            >
                              Accept
                            </Button>
                            <Button
                              size="sm"
                              variant="ghost"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`cancel-claim-${claim.id}`, () =>
                                  orderAfterSales.cancelClaim(client, order.id, claim.id),
                                )
                              }
                            >
                              Cancel
                            </Button>
                          </>
                        ) : null}
                        {claim.status === "accepted" ? (
                          <>
                            {claim.items.some((item) => item.item_type === "replace") ? (
                              <Button
                                size="sm"
                                disabled={recordBusy !== null}
                                onClick={() =>
                                  void runRecordAction(`fulfill-claim-${claim.id}`, () =>
                                    orderAfterSales.fulfillClaim(client, order.id, claim.id),
                                  )
                                }
                              >
                                Fulfill replacement
                              </Button>
                            ) : null}
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`refund-claim-${claim.id}`, () =>
                                  orderAfterSales.refundClaim(client, order.id, claim.id),
                                )
                              }
                            >
                              Refund
                            </Button>
                            {differenceActions("claim", claim.id, claim.difference_total)}
                          </>
                        ) : null}
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}

            {data.exchanges.length > 0 ? (
              <div>
                <p className="text-[11.5px] font-medium text-ink-soft">Exchanges</p>
                <ul className="mt-2 divide-y divide-border/70">
                  {data.exchanges.map((exchange) => (
                    <li key={exchange.id} className="py-3 first:pt-0 last:pb-0">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-mono text-[10px] text-faint">
                          #{shortId(exchange.id)}
                        </span>
                        <AfterSaleStatus status={exchange.status} />
                      </div>
                      <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted-foreground">
                        {exchange.reason ?? "No reason recorded"} ·{" "}
                        {unitCount(exchange.inbound_items)} in, {unitCount(exchange.outbound_items)}{" "}
                        out
                      </p>
                      {differenceNote(exchange.difference_total)}
                      <div className="mt-2 flex flex-wrap gap-1.5">
                        {exchange.status === "requested" && canWrite ? (
                          <>
                            <Button
                              size="sm"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`accept-exchange-${exchange.id}`, () =>
                                  orderAfterSales.acceptExchange(client, order.id, exchange.id),
                                )
                              }
                            >
                              Accept
                            </Button>
                            <Button
                              size="sm"
                              variant="ghost"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`cancel-exchange-${exchange.id}`, () =>
                                  orderAfterSales.cancelExchange(client, order.id, exchange.id),
                                )
                              }
                            >
                              Cancel
                            </Button>
                          </>
                        ) : null}
                        {exchange.status === "accepted" ? (
                          <>
                            <Button
                              size="sm"
                              disabled={recordBusy !== null}
                              onClick={() =>
                                void runRecordAction(`fulfill-exchange-${exchange.id}`, () =>
                                  orderAfterSales.fulfillExchange(client, order.id, exchange.id),
                                )
                              }
                            >
                              Fulfill replacement
                            </Button>
                            {differenceActions("exchange", exchange.id, exchange.difference_total)}
                          </>
                        ) : null}
                      </div>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
          </div>
        )}
      </section>

      <Dialog
        open={creation !== null}
        onOpenChange={(open) => !open && !creating && setCreation(null)}
      >
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>
              {creation ? CREATION_LABELS[creation] : "After-sales"} - order #{order.number}
            </DialogTitle>
            <DialogDescription>{creation ? CREATION_HINTS[creation] : ""}</DialogDescription>
          </DialogHeader>
          <div className="max-h-[55vh] space-y-3 overflow-y-auto pr-1">
            {order.items.map((item) => {
              const key = item.id ?? item.sku;
              return (
                <div key={key} className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <p className="truncate text-[12.5px] font-medium text-foreground">
                      {item.title}
                    </p>
                    <p className="mt-0.5 font-mono text-[10px] text-faint">
                      {item.sku} · up to {item.quantity}
                    </p>
                  </div>
                  <Input
                    type="number"
                    min={0}
                    max={item.quantity}
                    aria-label={`Quantity for ${item.title}`}
                    className="w-20"
                    value={draftQuantities[key] ?? 0}
                    onChange={(event) =>
                      setDraftQuantities((current) => ({
                        ...current,
                        [key]: Number(event.target.value),
                      }))
                    }
                    disabled={creating}
                  />
                </div>
              );
            })}
            {creation === "exchange" ? (
              <div className="space-y-2">
                <Label htmlFor="replacement-variant">Replacement variant id</Label>
                <Input
                  id="replacement-variant"
                  placeholder="variant_..."
                  value={replacementVariant}
                  onChange={(event) => setReplacementVariant(event.target.value)}
                  disabled={creating}
                />
                <p className="text-[11px] text-muted-foreground">
                  Every exchanged unit ships as this variant.
                </p>
              </div>
            ) : null}
            <div className="space-y-2">
              <Label htmlFor="after-sales-reason">Reason</Label>
              <Input
                id="after-sales-reason"
                placeholder="Customer changed their mind"
                value={draftReason}
                onChange={(event) => setDraftReason(event.target.value)}
                disabled={creating}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setCreation(null)} disabled={creating}>
              Cancel
            </Button>
            <Button onClick={() => void submitCreation()} disabled={creating}>
              {creating ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {creation ? CREATION_LABELS[creation] : "Save"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={collectTarget !== null}
        onOpenChange={(open) => !open && setCollectTarget(null)}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Collect difference - order #{order.number}</DialogTitle>
            <DialogDescription>
              Opens a payable for the difference the customer still owes.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="difference-provider">Payment provider</Label>
            <Input
              id="difference-provider"
              placeholder="manual"
              value={provider}
              onChange={(event) => setProvider(event.target.value)}
              disabled={recordBusy !== null}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setCollectTarget(null)}>
              Cancel
            </Button>
            <Button onClick={() => void submitCollect()} disabled={recordBusy !== null}>
              Collect difference
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
