import { useCallback, useEffect, useState } from "react";
import { Gift, Loader2, Plus, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import type { GiftCard, GiftCardDetail } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
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
import { demoGiftCards, demoGiftCardRedemptions } from "@/lib/demo-commerce";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  getGiftCard,
  giftCardInputError,
  giftCardStatusLabel,
  issueGiftCard,
  listGiftCards,
  remainingFraction,
  setGiftCardEnabled,
} from "@/lib/gift-cards";
import { PERMISSIONS } from "@/lib/permissions";
import { formatMinorAmount } from "@/lib/pricing";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 20;

export function GiftCardsPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoCards, setDemoCards] = useState<GiftCard[]>(demoGiftCards);
  const [page, setPage] = useState(1);
  const [issueOpen, setIssueOpen] = useState(false);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [detail, setDetail] = useState<GiftCard | null>(null);

  const canWrite = can(PERMISSIONS.payments.write);

  useEffect(() => {
    setDemoCards(demoGiftCards());
    setPage(1);
    setDetail(null);
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<{ data: GiftCard[]; total: number }> => {
      if (mode === "demo") {
        const cards = demoCards;
        const offset = (page - 1) * PAGE_SIZE;
        return Promise.resolve({
          data: cards.slice(offset, offset + PAGE_SIZE),
          total: cards.length,
        });
      }
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listGiftCards(
        store.id,
        { limit: PAGE_SIZE, offset: (page - 1) * PAGE_SIZE },
        signal,
      ).then((result) => ({ data: result.data, total: result.pagination.total }));
    },
    [mode, store, demoCards, page],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoCards,
    page,
  ]);

  const toggle = async (card: GiftCard) => {
    if (busyId) return;
    setBusyId(card.id);
    try {
      if (mode === "demo") {
        setDemoCards((current) =>
          current.map((entry) =>
            entry.id === card.id
              ? {
                  ...entry,
                  status: entry.status === "active" ? ("disabled" as const) : ("active" as const),
                  updated_at: Math.floor(Date.now() / 1000),
                }
              : entry,
          ),
        );
        toast.success("Gift card updated in the demo");
        return;
      }
      if (!store) throw new Error("No store is selected.");
      const updated = await setGiftCardEnabled(store.id, card.id, card.status !== "active");
      toast.success(updated.status === "active" ? "Gift card enabled" : "Gift card disabled");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const rows = data.data;
  const total = data.total;
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const currentPage = Math.min(page, pages);

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Gift cards"
        description="Issue prepaid balances, review what remains, and disable a card that should stop being redeemable."
        actions={
          <div className="flex items-center gap-2">
            <Button variant="outline" onClick={reload} disabled={loading}>
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
              Refresh
            </Button>
            {canWrite ? (
              <Button onClick={() => setIssueOpen(true)}>
                <Plus className="size-4" aria-hidden="true" />
                Issue gift card
              </Button>
            ) : null}
          </div>
        }
      />

      <Card className="overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Code</TableHead>
              <TableHead className="w-[140px] text-right">Balance</TableHead>
              <TableHead className="hidden w-[160px] md:table-cell">Issued</TableHead>
              <TableHead className="hidden w-[130px] lg:table-cell">Updated</TableHead>
              <TableHead className="w-[110px]">State</TableHead>
              <TableHead className="w-[110px] pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((card) => (
              <TableRow key={card.id} className="h-[58px]">
                <TableCell className="pl-6">
                  <button
                    type="button"
                    onClick={() => setDetail(card)}
                    className="cursor-pointer text-left font-mono text-[12px] font-medium tracking-[0.08em] text-foreground underline-offset-4 hover:underline"
                    aria-label={`Open gift card ${card.code}`}
                  >
                    {card.code}
                  </button>
                  <p className="mt-1 text-[10.5px] text-faint">{card.id}</p>
                </TableCell>
                <TableCell className="text-right">
                  <p className="text-[12.5px] font-medium text-foreground tnum">
                    {formatMinorAmount(card.balance_minor, card.currency_code)}
                  </p>
                  <p className="text-[10.5px] text-faint tnum">
                    of {formatMinorAmount(card.initial_amount_minor, card.currency_code)}
                  </p>
                  <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-secondary">
                    <div
                      className={cn(
                        "h-full rounded-full",
                        remainingFraction(card) === 0 ? "bg-faint/40" : "bg-brand",
                      )}
                      style={{ width: `${Math.round(remainingFraction(card) * 100)}%` }}
                    />
                  </div>
                </TableCell>
                <TableCell className="hidden text-[11.5px] text-muted-foreground md:table-cell">
                  {shortDate(fromUnixSeconds(card.created_at))}
                </TableCell>
                <TableCell className="hidden text-[11.5px] text-muted-foreground lg:table-cell">
                  {shortDate(fromUnixSeconds(card.updated_at))}
                </TableCell>
                <TableCell>
                  <Badge variant={card.status === "active" ? "brand" : "secondary"}>
                    {giftCardStatusLabel(card.status)}
                  </Badge>
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={!canWrite || busyId !== null}
                    onClick={() => void toggle(card)}
                  >
                    {busyId === card.id ? (
                      <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                    ) : null}
                    {card.status === "active" ? "Disable" : "Enable"}
                  </Button>
                </TableCell>
              </TableRow>
            ))}
            {rows.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={6} className="p-0">
                  <EmptyState
                    title="No gift cards yet"
                    description="Issue a card when a customer buys one at the counter or needs a store-credit balance."
                    action={
                      canWrite ? (
                        <Button onClick={() => setIssueOpen(true)}>
                          <Plus className="size-4" aria-hidden="true" />
                          Issue gift card
                        </Button>
                      ) : null
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>

        <div className="flex flex-col gap-3 border-t border-border px-6 py-3.5 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[11.5px] text-muted-foreground">
            {total === 0
              ? "No gift cards to show"
              : `Showing ${(currentPage - 1) * PAGE_SIZE + 1}–${(currentPage - 1) * PAGE_SIZE + rows.length} of ${total.toLocaleString("en-US")}`}
            {mode === "demo" ? " · sample data" : ""}
          </p>
          <Pagination page={currentPage} pages={pages} onChange={setPage} />
        </div>
      </Card>

      {issueOpen ? (
        <IssueGiftCardDialog
          currency={store?.currency_code ?? "USD"}
          onClose={() => setIssueOpen(false)}
          onIssued={(card) => {
            setIssueOpen(false);
            if (mode === "demo") {
              setDemoCards((current) => [card, ...current]);
              return;
            }
            reload();
          }}
        />
      ) : null}

      <GiftCardDetailSheet
        card={detail}
        onOpenChange={(open) => {
          if (!open) setDetail(null);
        }}
      />
    </div>
  );
}

function IssueGiftCardDialog({
  currency,
  onClose,
  onIssued,
}: {
  currency: string;
  onClose: () => void;
  onIssued: (card: GiftCard) => void;
}) {
  const { mode, store } = useAuth();
  const [amount, setAmount] = useState("");
  const [currencyCode, setCurrencyCode] = useState(currency);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [fieldError, setFieldError] = useState<string | null>(null);

  const submit = async () => {
    if (busy) return;
    const message = giftCardInputError({ amount, currency: currencyCode, code });
    if (message !== null) {
      setFieldError(message);
      return;
    }
    setBusy(true);
    try {
      const amountMinor = Math.round(Number(amount) * 100);
      const trimmedCode = code.trim();
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        onIssued({
          id: `gift_demo_${Date.now()}`,
          code: trimmedCode.toUpperCase() || "DEMO0000DEMO0000",
          currency_code: currencyCode.trim().toUpperCase(),
          initial_amount_minor: amountMinor,
          balance_minor: amountMinor,
          status: "active",
          metadata: {},
          created_at: now,
          updated_at: now,
        });
        toast.success("Gift card issued in the demo");
        return;
      }
      if (!store) throw new Error("No store is selected.");
      const card = await issueGiftCard(store.id, {
        currency_code: currencyCode.trim().toUpperCase(),
        amount_minor: amountMinor,
        ...(trimmedCode.length > 0 ? { code: trimmedCode } : {}),
      });
      toast.success(`Gift card ${card.code} issued`);
      onIssued(card);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && !busy && onClose()}>
      <DialogContent className="sm:max-w-md">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>Issue a gift card</DialogTitle>
            <DialogDescription>
              The card starts active with the full amount. Codes are bearer credentials, so hand
              them to the buyer directly.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="gift-card-amount">Amount</Label>
              <Input
                id="gift-card-amount"
                inputMode="decimal"
                placeholder="50.00"
                value={amount}
                onChange={(event) => {
                  setAmount(event.target.value);
                  setFieldError(null);
                }}
                autoFocus
                disabled={busy}
                aria-invalid={fieldError !== null}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="gift-card-currency">Currency</Label>
              <Input
                id="gift-card-currency"
                value={currencyCode}
                onChange={(event) => {
                  setCurrencyCode(event.target.value.toUpperCase());
                  setFieldError(null);
                }}
                placeholder="USD"
                maxLength={3}
                spellCheck={false}
                disabled={busy}
                className="font-mono uppercase"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="gift-card-code">Code (optional)</Label>
            <Input
              id="gift-card-code"
              value={code}
              onChange={(event) => {
                setCode(event.target.value);
                setFieldError(null);
              }}
              placeholder="Generated when left blank"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              className="font-mono"
            />
            {fieldError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {fieldError}
              </p>
            ) : null}
          </div>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Gift className="size-4" aria-hidden="true" />
              )}
              Issue card
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function GiftCardDetailSheet({
  card,
  onOpenChange,
}: {
  card: GiftCard | null;
  onOpenChange: (open: boolean) => void;
}) {
  const { mode, store } = useAuth();
  const [detail, setDetail] = useState<GiftCardDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setDetail(null);
    setError(null);
    if (card === null || mode === "demo") {
      setLoading(false);
      return;
    }
    if (!store) return;
    const controller = new AbortController();
    setLoading(true);
    void getGiftCard(store.id, card.id, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) setDetail(result);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setError(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [card, mode, store]);

  const demoDetail =
    mode === "demo" && card !== null
      ? { ...card, redemptions: demoGiftCardRedemptions(card.id) }
      : null;
  const current = detail ?? demoDetail ?? card;
  const redemptions = detail?.redemptions ?? demoDetail?.redemptions ?? [];

  return (
    <Sheet
      open={card !== null}
      onOpenChange={(open) => {
        if (!open) onOpenChange(false);
      }}
    >
      <SheetContent className="flex flex-col sm:max-w-2xl">
        {current ? (
          <>
            <SheetHeader>
              <div className="flex flex-wrap items-center gap-2 pr-8">
                <Badge variant={current.status === "active" ? "brand" : "secondary"}>
                  {giftCardStatusLabel(current.status)}
                </Badge>
                <span className="font-mono text-[10.5px] text-faint">{current.id}</span>
              </div>
              <SheetTitle className="font-mono text-[16px] tracking-[0.08em]">
                {current.code}
              </SheetTitle>
              <SheetDescription>
                {formatMinorAmount(current.balance_minor, current.currency_code)} remaining of{" "}
                {formatMinorAmount(current.initial_amount_minor, current.currency_code)} issued.
              </SheetDescription>
            </SheetHeader>

            <div className="flex-1 space-y-6 overflow-y-auto p-6">
              <section className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-[11px] font-medium text-muted-foreground">Balance</p>
                  <p className="mt-1 text-[12.5px] text-ink-soft tnum">
                    {formatMinorAmount(current.balance_minor, current.currency_code)}
                  </p>
                </div>
                <div>
                  <p className="text-[11px] font-medium text-muted-foreground">Initial amount</p>
                  <p className="mt-1 text-[12.5px] text-ink-soft tnum">
                    {formatMinorAmount(current.initial_amount_minor, current.currency_code)}
                  </p>
                </div>
                <div>
                  <p className="text-[11px] font-medium text-muted-foreground">Issued</p>
                  <p className="mt-1 text-[12.5px] text-ink-soft">
                    {shortDate(fromUnixSeconds(current.created_at))}
                  </p>
                </div>
                <div>
                  <p className="text-[11px] font-medium text-muted-foreground">Last updated</p>
                  <p className="mt-1 text-[12.5px] text-ink-soft">
                    {shortDate(fromUnixSeconds(current.updated_at))}
                  </p>
                </div>
              </section>

              <section>
                <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
                  Redemptions
                </p>
                {error ? (
                  <p className="mt-2 text-[12px] text-destructive">{error}</p>
                ) : loading ? (
                  <div className="flex justify-center py-8">
                    <Loader2 className="size-4 animate-spin text-faint" aria-hidden="true" />
                  </div>
                ) : redemptions.length === 0 ? (
                  <p className="mt-2 rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12px] text-muted-foreground">
                    No redemptions recorded for this card.
                  </p>
                ) : (
                  <div className="mt-2 overflow-hidden rounded-lg border border-border">
                    <Table>
                      <TableHeader>
                        <TableRow className="hover:bg-transparent">
                          <TableHead>Amount</TableHead>
                          <TableHead className="w-[110px]">Status</TableHead>
                          <TableHead>Order</TableHead>
                          <TableHead className="w-[120px]">Created</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {redemptions.map((redemption) => (
                          <TableRow key={redemption.id}>
                            <TableCell className="tnum">
                              {formatMinorAmount(redemption.amount_minor, current.currency_code)}
                            </TableCell>
                            <TableCell>
                              <Badge
                                variant={redemption.status === "redeemed" ? "brand" : "warning"}
                              >
                                {redemption.status === "redeemed" ? "Redeemed" : "Reversed"}
                              </Badge>
                            </TableCell>
                            <TableCell className="font-mono text-[11px] text-ink-soft">
                              {redemption.order_id ?? "—"}
                            </TableCell>
                            <TableCell className="text-[12px] text-muted-foreground">
                              {shortDate(fromUnixSeconds(redemption.created_at))}
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
  );
}
