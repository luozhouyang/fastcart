import { useCallback, useEffect, useState } from "react";
import { Bell, History, Loader2, Pencil, Plus, Send, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type {
  NotificationChannel,
  NotificationChannelDelivery,
  UpdateNotificationChannelRequest,
} from "@fastcart/shared-types";

import {
  NotificationChannelDialog,
  type ChannelDraft,
} from "@/components/notification-channel-dialog";
import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, PageSkeleton } from "@/components/states";
import { DeliveryStatusPill } from "@/components/status-pill";
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
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { useAuth } from "@/lib/auth";
import { errorMessage } from "@/lib/api";
import {
  demoNotificationChannels,
  demoNotificationDeliveries,
  demoNotificationEvents,
} from "@/lib/demo-inquiries";
import { fromUnixSeconds, shortDate, timeOfDay } from "@/lib/format";
import { loadNotificationDeliveries, loadNotifications } from "@/lib/loaders";
import {
  channelEndpointLabel,
  channelSecretLabel,
  eventLabel,
  maskEndpoint,
} from "@/lib/notifications";
import { useAsyncData } from "@/lib/use-async-data";

export function NotificationsPage() {
  const { mode, client, store, dataKey } = useAuth();
  const [demoChannels, setDemoChannels] = useState<NotificationChannel[]>(() =>
    demoNotificationChannels(),
  );
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<NotificationChannel | null>(null);
  const [busy, setBusy] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<NotificationChannel | null>(null);
  const [deliveriesChannel, setDeliveriesChannel] = useState<NotificationChannel | null>(null);
  const [deliveries, setDeliveries] = useState<NotificationChannelDelivery[]>([]);
  const [deliveriesLoading, setDeliveriesLoading] = useState(false);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") {
        return Promise.resolve({
          channels: demoNotificationChannels(),
          events: demoNotificationEvents(),
        });
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadNotifications(client, signal);
    },
    [mode, client],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);

  const channels = mode === "demo" ? demoChannels : (data?.channels ?? []);
  const events = mode === "demo" ? demoNotificationEvents() : (data?.events ?? []);

  useEffect(() => {
    if (!deliveriesChannel) {
      setDeliveries([]);
      return;
    }
    if (mode === "demo") {
      setDeliveries(demoNotificationDeliveries(deliveriesChannel.id));
      return;
    }
    if (!client) return;
    const controller = new AbortController();
    setDeliveriesLoading(true);
    loadNotificationDeliveries(client, deliveriesChannel.id, controller.signal)
      .then((result) => setDeliveries(result))
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) toast.error(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setDeliveriesLoading(false);
      });
    return () => controller.abort();
  }, [deliveriesChannel, mode, client]);

  const openCreate = () => {
    setEditing(null);
    setDialogOpen(true);
  };

  const openEdit = (channel: NotificationChannel) => {
    setEditing(channel);
    setDialogOpen(true);
  };

  const save = async (draft: ChannelDraft) => {
    if (busy) return;
    setBusy(true);
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editing) {
          setDemoChannels((current) =>
            current.map((channel) =>
              channel.id === editing.id
                ? {
                    ...channel,
                    name: draft.name.trim(),
                    events: draft.events,
                    is_active: draft.isActive,
                    updated_at: now,
                    endpoint_url_masked:
                      draft.endpointUrl.trim().length > 0
                        ? maskEndpoint(draft.endpointUrl.trim())
                        : channel.endpoint_url_masked,
                    signing_secret_configured:
                      draft.signingSecret.length > 0 || channel.signing_secret_configured,
                  }
                : channel,
            ),
          );
        } else {
          const channel: NotificationChannel = {
            id: `nch_demo_${now}`,
            kind: "feishu",
            name: draft.name.trim(),
            events: draft.events,
            is_active: draft.isActive,
            config: {},
            endpoint_url_masked: maskEndpoint(draft.endpointUrl.trim()),
            signing_secret_configured: draft.signingSecret.length > 0,
            last_error: null,
            last_success_at: null,
            created_at: now,
            updated_at: now,
          };
          setDemoChannels((current) => [channel, ...current]);
        }
        setDialogOpen(false);
        toast.success(editing ? "Channel updated in the demo" : "Channel created in the demo");
      } else {
        if (!client) throw new Error("No store is selected.");
        if (editing) {
          const body: UpdateNotificationChannelRequest = {
            name: draft.name.trim(),
            events: draft.events,
            is_active: draft.isActive,
          };
          if (draft.endpointUrl.trim().length > 0) body.endpoint_url = draft.endpointUrl.trim();
          if (draft.signingSecret.length > 0) body.signing_secret = draft.signingSecret;
          await client.notificationChannels.update(editing.id, body);
        } else {
          await client.notificationChannels.create({
            kind: "feishu",
            name: draft.name.trim(),
            endpoint_url: draft.endpointUrl.trim(),
            events: draft.events,
            is_active: draft.isActive,
            ...(draft.signingSecret.length > 0 ? { signing_secret: draft.signingSecret } : {}),
          });
        }
        setDialogOpen(false);
        reload();
        toast.success(editing ? "Channel updated" : "Channel created");
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const test = async (channel: NotificationChannel) => {
    try {
      if (mode === "demo") {
        toast.success("Test notification delivered in the demo");
        return;
      }
      if (!client) throw new Error("No store is selected.");
      const result = await client.notificationChannels.test(channel.id);
      if (result.ok) {
        toast.success(result.message ?? "Test notification delivered");
      } else {
        toast.error(result.error ?? result.message ?? "The test notification failed.");
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    }
  };

  const remove = async () => {
    if (!deleteTarget || busy) return;
    setBusy(true);
    try {
      if (mode === "demo") {
        setDemoChannels((current) => current.filter((channel) => channel.id !== deleteTarget.id));
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.notificationChannels.remove(deleteTarget.id);
        reload();
      }
      toast.success("Channel deleted");
      setDeleteTarget(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  if (!data && mode !== "demo" && loading) return <PageSkeleton />;
  if (!data && mode !== "demo" && error) return <ErrorState message={error} onRetry={reload} />;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Notifications"
        description="Route order and inquiry events to Feishu. Delivery logs show what was sent and why it failed."
        actions={
          <Button onClick={openCreate}>
            <Plus className="size-4" aria-hidden="true" />
            New channel
          </Button>
        }
      />

      {channels.length === 0 ? (
        <Card>
          <EmptyState
            title="No notification channels"
            description="Create a channel and choose which store events it should receive."
            action={
              <Button onClick={openCreate}>
                <Plus className="size-4" aria-hidden="true" />
                New channel
              </Button>
            }
          />
        </Card>
      ) : (
        <div className="grid gap-3 xl:grid-cols-2">
          {channels.map((channel) => (
            <Card key={channel.id} className="p-5">
              <div className="flex items-start justify-between gap-3">
                <div className="flex min-w-0 items-center gap-3">
                  <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-brand-ink">
                    <Bell className="size-4" aria-hidden="true" />
                  </span>
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <p className="truncate text-[13px] font-medium text-foreground">
                        {channel.name}
                      </p>
                      <Badge variant="outline" className="capitalize">
                        {channel.kind}
                      </Badge>
                    </div>
                    <p className="mt-1 truncate font-mono text-[10.5px] text-faint">
                      {channelEndpointLabel(channel)}
                    </p>
                  </div>
                </div>
                <Badge variant={channel.is_active ? "brand" : "secondary"}>
                  {channel.is_active ? "Active" : "Inactive"}
                </Badge>
              </div>

              <div className="mt-4 space-y-2.5">
                <div className="flex flex-wrap gap-1.5">
                  {channel.events.map((type) => (
                    <Badge key={type} variant="secondary">
                      {eventLabel(events, type)}
                    </Badge>
                  ))}
                  {channel.events.length === 0 ? (
                    <span className="text-[11px] text-faint">No events selected</span>
                  ) : null}
                </div>

                <p className="text-[10.5px] text-faint">
                  {channelSecretLabel(channel.signing_secret_configured)}
                </p>

                {channel.last_error ? (
                  <p className="rounded-lg border border-destructive/20 bg-destructive/5 px-2.5 py-1.5 text-[10.5px] text-destructive">
                    Last error: {channel.last_error}
                  </p>
                ) : channel.last_success_at ? (
                  <p className="text-[10.5px] text-muted-foreground">
                    Last delivered {shortDate(fromUnixSeconds(channel.last_success_at))} at{" "}
                    {timeOfDay(fromUnixSeconds(channel.last_success_at))}
                  </p>
                ) : (
                  <p className="text-[10.5px] text-faint">No deliveries yet</p>
                )}
              </div>

              <div className="mt-4 flex flex-wrap items-center gap-1.5">
                <Button variant="outline" size="sm" onClick={() => void test(channel)}>
                  <Send className="size-3.5" aria-hidden="true" />
                  Test
                </Button>
                <Button variant="outline" size="sm" onClick={() => setDeliveriesChannel(channel)}>
                  <History className="size-3.5" aria-hidden="true" />
                  Deliveries
                </Button>
                <Button variant="ghost" size="sm" onClick={() => openEdit(channel)}>
                  <Pencil className="size-3.5" aria-hidden="true" />
                  Edit
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="ml-auto text-destructive hover:text-destructive"
                  onClick={() => setDeleteTarget(channel)}
                >
                  <Trash2 className="size-3.5" aria-hidden="true" />
                  Delete
                </Button>
              </div>
            </Card>
          ))}
        </div>
      )}

      <NotificationChannelDialog
        open={dialogOpen}
        channel={editing}
        events={events}
        busy={busy}
        onOpenChange={setDialogOpen}
        onSubmit={(draft) => void save(draft)}
      />

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {deleteTarget?.name}?</DialogTitle>
            <DialogDescription>
              Deliveries and configuration for this channel are removed. This cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTarget(null)} disabled={busy}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void remove()} disabled={busy}>
              Delete channel
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Sheet
        open={deliveriesChannel !== null}
        onOpenChange={(open) => !open && setDeliveriesChannel(null)}
      >
        <SheetContent side="right" className="sm:max-w-lg">
          <SheetHeader>
            <SheetTitle>Deliveries</SheetTitle>
            <SheetDescription>
              {deliveriesChannel ? `${deliveriesChannel.name} · last 50 attempts` : ""}
            </SheetDescription>
          </SheetHeader>
          <div className="flex-1 overflow-y-auto p-6">
            {deliveriesLoading ? (
              <div className="flex justify-center py-12">
                <Loader2 className="size-5 animate-spin text-faint" aria-hidden="true" />
              </div>
            ) : deliveries.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border px-3 py-8 text-center text-[12px] text-muted-foreground">
                No deliveries recorded yet.
              </p>
            ) : (
              <ul className="space-y-2">
                {deliveries.map((delivery) => (
                  <li
                    key={delivery.id}
                    className="rounded-xl border border-border bg-background p-3"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <DeliveryStatusPill status={delivery.status} />
                      <span className="font-mono text-[10px] text-faint">
                        {delivery.event_type}
                      </span>
                    </div>
                    <div className="mt-2 grid grid-cols-3 gap-2 text-[10.5px] text-muted-foreground">
                      <span>Attempts: {delivery.attempts}</span>
                      <span>HTTP: {delivery.response_status ?? "—"}</span>
                      <span className="text-right">
                        {shortDate(fromUnixSeconds(delivery.created_at))}
                      </span>
                    </div>
                    {delivery.last_error ? (
                      <p className="mt-1.5 text-[10.5px] text-destructive">{delivery.last_error}</p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </SheetContent>
      </Sheet>
    </div>
  );
}
