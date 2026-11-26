import { Check, Copy } from "lucide-react";
import { useState } from "react";
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

/**
 * Shows a signing secret exactly once after it is created or rotated. The
 * secret is never fetched again by the API, so closing this dialog discards it.
 */
export function WebhookSecretDialog({
  open,
  secret,
  endpointUrl,
  onOpenChange,
}: {
  open: boolean;
  secret: string | null;
  endpointUrl: string;
  onOpenChange: (open: boolean) => void;
}) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    if (secret === null) return;
    try {
      await navigator.clipboard.writeText(secret);
      setCopied(true);
      toast.success("Secret copied");
    } catch {
      toast.error("The clipboard is unavailable. Select the secret and copy it manually.");
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) setCopied(false);
        onOpenChange(next);
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Signing secret</DialogTitle>
          <DialogDescription>
            Use this secret to verify the x-fastcart-signature header on {endpointUrl}. It is shown
            once and cannot be retrieved later.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-2">
          <div className="flex items-center gap-2 rounded-lg border border-border bg-secondary/40 p-3">
            <code className="min-w-0 flex-1 break-all font-mono text-[12px] text-foreground">
              {secret ?? ""}
            </code>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void copy()}
              disabled={secret === null}
            >
              {copied ? (
                <Check className="size-3.5" aria-hidden="true" />
              ) : (
                <Copy className="size-3.5" aria-hidden="true" />
              )}
              {copied ? "Copied" : "Copy"}
            </Button>
          </div>
          <p className="text-[11px] leading-relaxed text-faint">
            Store the secret in your endpoint's environment now. Rotating it invalidates the
            previous secret immediately.
          </p>
        </div>

        <DialogFooter>
          <Button onClick={() => onOpenChange(false)}>Done</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
