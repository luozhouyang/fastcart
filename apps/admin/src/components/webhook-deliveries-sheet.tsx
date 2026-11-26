import { useCallback, useEffect, useState } from "react";
import { History, Loader2, Play, RefreshCw } from "lucide-react";
import { toast } from "sonner";

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
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { demoWebhookDeliveries } from "@/lib/demo-commerce";
import { fromUnixSeconds, shortDate, timeOfDay } from "@/lib/format";
import { PERMISSIONS } from "@/lib/permissions";
import {
  WEBHOOK_DELIVERY_LABELS,
  WEBHOOK_DELIVERY_TONES,
  canReplayDelivery,
  listWebhookDeliveries,
  replayWebhookDelivery,
  type WebhookDelivery,
  type WebhookEndpoint,
} from "@/lib/webhooks";
import { cn } from "@/lib/utils";

const DOT_TONES: Record<string, string> = {
  brand: "bg-brand",
  info: "bg-info",
  destructive: "bg-destructive",
  secondary: "bg-faint",
};

export function WebhookDeliveriesSheet({
  endpoint,
  onOpenChange,
}: {
  endpoint: WebhookEndpoint | null;
  onOpenChange: (open: boolean) => void;
}) {
  const { mode, store, can } = useAuth();
  const [deliveries, setDeliveries] = useState<WebhookDelivery[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [version, setVersion] = useState(0);
  const [replayingId, setReplayingId] = useState<string | null>(null);
  const endpointId = endpoint?.id ?? null;

  useEffect(() => {
    if (endpointId === null) return;
    setError(null);
    if (mode === "demo") {
      setDeliveries(demoWebhookDeliveries(endpointId));
      setLoading(false);
      return;
    }
    if (!store) return;
    const controller = new AbortController();
    setLoading(true);
    void listWebhookDeliveries(store.id, endpointId, { limit: 50 }, controller.signal)
      .then((rows) => {
        if (!controller.signal.aborted) setDeliveries(rows);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) setError(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [endpointId, mode, store, version]);

  const replay = useCallback(
    async (delivery: WebhookDelivery) => {
      if (replayingId !== null) return;
      setReplayingId(delivery.id);
      try {
        if (mode === "demo") {
          setDeliveries((current) =>
            current.map((row) =>
              row.id === delivery.id
                ? { ...row, status: "pending", attempts: 0, response_status: null }
                : row,
            ),
          );
          toast.success("Delivery requeued in the demo");
        } else {
          if (!store) throw new Error("No store is selected.");
          const updated = await replayWebhookDelivery(store.id, delivery.id);
          setDeliveries((current) => current.map((row) => (row.id === updated.id ? updated : row)));
          toast.success("Delivery requeued");
        }
      } catch (cause) {
        toast.error(errorMessage(cause));
      } finally {
        setReplayingId(null);
      }
    },
    [mode, store, replayingId],
  );

  return (
    <Sheet
      open={endpoint !== null}
      onOpenChange={(open) => {
        if (!open) setDeliveries([]);
        onOpenChange(open);
      }}
    >
      <SheetContent className="flex flex-col sm:max-w-2xl">
        <SheetHeader>
          <SheetTitle>Deliveries</SheetTitle>
          <SheetDescription>
            {endpoint ? `${endpoint.url} · last 50 deliveries` : ""}
          </SheetDescription>
        </SheetHeader>

        <div className="flex-1 overflow-y-auto p-6">
          <div className="mb-4 flex items-center justify-between">
            <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
              Recent attempts
            </p>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setVersion((value) => value + 1)}
              disabled={loading}
            >
              <RefreshCw className={cn("size-3.5", loading && "animate-spin")} aria-hidden="true" />
              Refresh
            </Button>
          </div>

          {error ? (
            <p className="rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">
              {error}
            </p>
          ) : loading ? (
            <div className="flex justify-center py-12">
              <Loader2 className="size-5 animate-spin text-faint" aria-hidden="true" />
            </div>
          ) : deliveries.length === 0 ? (
            <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed border-border px-4 py-12 text-center">
              <History className="size-5 text-faint" aria-hidden="true" />
              <p className="text-[12.5px] text-muted-foreground">
                No deliveries recorded yet. Events appear here once this endpoint is subscribed.
              </p>
            </div>
          ) : (
            <ul className="space-y-2">
              {deliveries.map((delivery) => {
                const tone = WEBHOOK_DELIVERY_TONES[delivery.status];
                const replayable =
                  can(PERMISSIONS.settings.write) && canReplayDelivery(delivery.status);
                return (
                  <li
                    key={delivery.id}
                    className="rounded-xl border border-border bg-background p-3"
                  >
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <Badge variant={tone}>
                          <span
                            className={cn("size-1.5 rounded-full", DOT_TONES[tone])}
                            aria-hidden="true"
                          />
                          {WEBHOOK_DELIVERY_LABELS[delivery.status]}
                        </Badge>
                        <span className="font-mono text-[10.5px] text-ink-soft">
                          {delivery.event_type}
                        </span>
                      </div>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => void replay(delivery)}
                        disabled={!replayable || replayingId !== null}
                      >
                        {replayingId === delivery.id ? (
                          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                        ) : (
                          <Play className="size-3.5" aria-hidden="true" />
                        )}
                        Replay
                      </Button>
                    </div>
                    <div className="mt-2 grid grid-cols-2 gap-x-3 gap-y-1 text-[10.5px] text-muted-foreground sm:grid-cols-4">
                      <span>Attempts: {delivery.attempts}</span>
                      <span>HTTP: {delivery.response_status ?? "—"}</span>
                      <span>
                        {shortDate(fromUnixSeconds(delivery.created_at))} at{" "}
                        {timeOfDay(fromUnixSeconds(delivery.created_at))}
                      </span>
                      <span className="truncate font-mono">{delivery.id}</span>
                    </div>
                    {delivery.response_body ? (
                      <pre className="mt-2 max-h-24 overflow-auto rounded-md bg-secondary/50 p-2 font-mono text-[10px] whitespace-pre-wrap text-ink-soft">
                        {delivery.response_body}
                      </pre>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <SheetFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}
