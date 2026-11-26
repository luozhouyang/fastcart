import { Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export type SessionAction = "capture" | "cancel" | "fail";

export const SESSION_ACTION_LABELS: Record<SessionAction, string> = {
  capture: "Capture",
  cancel: "Cancel",
  fail: "Fail",
};

export const SESSION_ACTION_PAST: Record<SessionAction, string> = {
  capture: "captured",
  cancel: "canceled",
  fail: "failed",
};

const SESSION_ACTION_DESCRIPTIONS: Record<SessionAction, string> = {
  capture: "Captures the remaining authorized amount and records a payment for the order.",
  cancel: "Cancels the session before any money is captured. Capture is no longer possible.",
  fail: "Marks the session as failed with a reason. The storefront must start a new session.",
};

/** Confirms one capture, cancel or fail transition for a payment session. */
export function PaymentSessionActionDialog({
  action,
  busy,
  onCancel,
  onConfirm,
}: {
  action: SessionAction | null;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog
      open={action !== null}
      onOpenChange={(open) => {
        if (!open && !busy) onCancel();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {action ? `${SESSION_ACTION_LABELS[action]} payment session?` : ""}
          </DialogTitle>
          <DialogDescription>{action ? SESSION_ACTION_DESCRIPTIONS[action] : ""}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button
            variant={action === "fail" ? "destructive" : "default"}
            onClick={onConfirm}
            disabled={busy || action === null}
          >
            {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
            {action ? SESSION_ACTION_LABELS[action] : ""}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
