import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import type { WebhookEndpoint } from "@fastcart/shared-types";

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
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/api";
import { eventsLabel, parseEventList, webhookUrlError } from "@/lib/webhooks";

export interface WebhookDraft {
  url: string;
  events: string[];
  isActive: boolean;
  secret: string | null;
}

export function WebhookDialog({
  open,
  endpoint,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  endpoint: WebhookEndpoint | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (draft: WebhookDraft) => Promise<void>;
}) {
  const [url, setUrl] = useState("");
  const [events, setEvents] = useState("");
  const [secret, setSecret] = useState("");
  const [isActive, setIsActive] = useState(true);
  const [urlError, setUrlError] = useState<string | null>(null);
  const [eventsError, setEventsError] = useState<string | null>(null);
  const [secretError, setSecretError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setUrl(endpoint?.url ?? "");
    setEvents(endpoint?.events.join(", ") ?? "");
    setSecret("");
    setIsActive(endpoint?.is_active ?? true);
    setUrlError(null);
    setEventsError(null);
    setSecretError(null);
    setBusy(false);
  }, [open, endpoint?.id]);

  const submit = async () => {
    if (busy) return;
    const problem = webhookUrlError(url);
    if (problem) {
      setUrlError(problem);
      return;
    }
    const parsed = parseEventList(events);
    if (parsed.length === 0) {
      setEventsError("Subscribe to at least one event, or use * for every event.");
      return;
    }
    const trimmedSecret = secret.trim();
    if (trimmedSecret.length > 0 && (trimmedSecret.length < 16 || trimmedSecret.length > 256)) {
      setSecretError("Secrets must be 16 to 256 characters with no whitespace.");
      return;
    }

    setBusy(true);
    try {
      await onSubmit({
        url: url.trim(),
        events: parsed,
        isActive,
        secret: trimmedSecret.length === 0 ? null : trimmedSecret,
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
      <DialogContent className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{endpoint ? "Edit webhook endpoint" : "New webhook endpoint"}</DialogTitle>
            <DialogDescription>
              Events are delivered as signed JSON payloads. Only https destinations are accepted.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="webhook-url">Endpoint URL</Label>
            <Input
              id="webhook-url"
              value={url}
              onChange={(event) => {
                setUrl(event.target.value);
                setUrlError(null);
              }}
              placeholder="https://example.com/hooks/fastcart"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              aria-invalid={urlError !== null}
              className="font-mono text-[12px]"
            />
            {urlError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {urlError}
              </p>
            ) : null}
          </div>

          <div className="space-y-2">
            <Label htmlFor="webhook-events">Events</Label>
            <Input
              id="webhook-events"
              value={events}
              onChange={(event) => {
                setEvents(event.target.value);
                setEventsError(null);
              }}
              placeholder="order.placed, payment.captured"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              aria-invalid={eventsError !== null}
              className="font-mono text-[12px]"
            />
            <p className="text-[11px] text-faint">
              Comma or space separated. Use <code className="font-mono">*</code> to receive every
              event.
            </p>
            {eventsError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {eventsError}
              </p>
            ) : null}
          </div>

          {endpoint === null ? (
            <div className="space-y-2">
              <Label htmlFor="webhook-secret">Signing secret</Label>
              <Input
                id="webhook-secret"
                value={secret}
                onChange={(event) => {
                  setSecret(event.target.value);
                  setSecretError(null);
                }}
                placeholder="Leave empty to generate one"
                autoComplete="new-password"
                spellCheck={false}
                disabled={busy}
                aria-invalid={secretError !== null}
                className="font-mono text-[12px]"
              />
              <p className="text-[11px] text-faint">
                Optional. A random secret is generated when this is empty.
              </p>
              {secretError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {secretError}
                </p>
              ) : null}
            </div>
          ) : (
            <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-secondary/40 px-4 py-3.5">
              <div>
                <Label htmlFor="webhook-active">Deliveries enabled</Label>
                <p className="mt-1 text-[11.5px] text-muted-foreground">
                  Disabled endpoints keep their configuration and delivery history.
                </p>
              </div>
              <Switch
                id="webhook-active"
                checked={isActive}
                onCheckedChange={setIsActive}
                disabled={busy}
              />
            </div>
          )}

          {endpoint !== null ? (
            <p className="text-[11px] text-faint">
              Current subscriptions: {eventsLabel(endpoint.events)}
            </p>
          ) : null}

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
              {endpoint ? "Save changes" : "Create endpoint"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
