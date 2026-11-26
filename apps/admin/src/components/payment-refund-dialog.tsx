import { useEffect, useState } from "react";
import { Loader2, Undo2 } from "lucide-react";
import { toast } from "sonner";

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
import { formatMinorAmount, formatMinorInput, parsePriceAmount } from "@/lib/pricing";

/** Collects a refund amount and reason for one payment. */
export function PaymentRefundDialog({
  open,
  maxMinor,
  currency,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  maxMinor: number;
  currency: string;
  onOpenChange: (open: boolean) => void;
  onSubmit: (amount: number, reason: string) => Promise<void>;
}) {
  const [amount, setAmount] = useState("");
  const [reason, setReason] = useState("");
  const [amountError, setAmountError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setAmount(maxMinor > 0 ? formatMinorInput(maxMinor) : "");
    setReason("");
    setAmountError(null);
    setBusy(false);
  }, [open, maxMinor]);

  const submit = async () => {
    if (busy) return;
    const parsed = parsePriceAmount(amount);
    if (!parsed.ok) {
      setAmountError(parsed.message);
      return;
    }
    if (parsed.minor <= 0) {
      setAmountError("Refund amount must be greater than zero.");
      return;
    }
    if (parsed.minor > maxMinor) {
      setAmountError(`The refund cannot exceed ${formatMinorAmount(maxMinor, currency)}.`);
      return;
    }
    setBusy(true);
    try {
      await onSubmit(parsed.minor, reason.trim());
      onOpenChange(false);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
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
            <DialogTitle>Create refund</DialogTitle>
            <DialogDescription>
              At most {formatMinorAmount(maxMinor, currency)} is still refundable. Refunds are sent
              to the payment provider immediately.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="refund-amount">Amount ({currency})</Label>
            <Input
              id="refund-amount"
              type="number"
              min="0"
              step="0.01"
              inputMode="decimal"
              value={amount}
              onChange={(event) => {
                setAmount(event.target.value);
                setAmountError(null);
              }}
              disabled={busy}
              aria-invalid={amountError !== null}
            />
            {amountError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {amountError}
              </p>
            ) : null}
          </div>

          <div className="space-y-2">
            <Label htmlFor="refund-reason">Reason</Label>
            <Input
              id="refund-reason"
              value={reason}
              onChange={(event) => setReason(event.target.value)}
              placeholder="Damaged in transit"
              autoComplete="off"
              disabled={busy}
            />
            <p className="text-[11px] text-faint">
              Shared with the provider and shown in the refund history.
            </p>
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
              {busy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Undo2 className="size-4" aria-hidden="true" />
              )}
              Refund
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
