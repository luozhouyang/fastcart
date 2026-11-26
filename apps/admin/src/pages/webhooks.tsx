import { useCallback, useEffect, useState } from "react";
import { Loader2, Plus, RefreshCw, RotateCw, Trash2, Webhook } from "lucide-react";
import { toast } from "sonner";
import type { WebhookEndpoint } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { ErrorState, TableSkeleton } from "@/components/states";
import { WebhookDeliveriesSheet } from "@/components/webhook-deliveries-sheet";
import { WebhookDialog, type WebhookDraft } from "@/components/webhook-dialog";
import { WebhookEndpointTable } from "@/components/webhook-endpoint-table";
import { WebhookSecretDialog } from "@/components/webhook-secret-dialog";
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
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { demoWebhookEndpoints } from "@/lib/demo-commerce";
import { PERMISSIONS } from "@/lib/permissions";
import { useAsyncData } from "@/lib/use-async-data";
import {
  createWebhook,
  deleteWebhook,
  listWebhooks,
  rotateWebhookSecret,
  updateWebhook,
} from "@/lib/webhooks";
import { cn } from "@/lib/utils";

export function WebhooksPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoEndpoints, setDemoEndpoints] = useState<WebhookEndpoint[]>(demoWebhookEndpoints);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<WebhookEndpoint | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<WebhookEndpoint | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [rotateTarget, setRotateTarget] = useState<WebhookEndpoint | null>(null);
  const [rotateBusy, setRotateBusy] = useState(false);
  const [deliveriesTarget, setDeliveriesTarget] = useState<WebhookEndpoint | null>(null);
  const [secret, setSecret] = useState<{ value: string; url: string } | null>(null);

  const canWrite = can(PERMISSIONS.settings.write);

  const loader = useCallback(
    (signal: AbortSignal): Promise<WebhookEndpoint[]> => {
      if (mode === "demo") return Promise.resolve(demoEndpoints);
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listWebhooks(store.id, signal);
    },
    [mode, store, demoEndpoints],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoEndpoints,
  ]);

  useEffect(() => {
    setDialogOpen(false);
    setEditing(null);
    setDeleteTarget(null);
    setRotateTarget(null);
    setDeliveriesTarget(null);
  }, [store?.id, dataKey]);

  const save = async (draft: WebhookDraft) => {
    if (mode === "demo") {
      const timestamp = Math.floor(Date.now() / 1000);
      if (editing) {
        setDemoEndpoints((current) =>
          current.map((endpoint) =>
            endpoint.id === editing.id
              ? {
                  ...endpoint,
                  url: draft.url,
                  events: draft.events,
                  is_active: draft.isActive,
                  updated_at: timestamp,
                }
              : endpoint,
          ),
        );
        toast.success("Webhook endpoint updated in the demo");
      } else {
        const created: WebhookEndpoint = {
          id: `wh_demo_${timestamp}`,
          url: draft.url,
          events: draft.events,
          is_active: true,
          metadata: {},
          created_at: timestamp,
          updated_at: timestamp,
        };
        setDemoEndpoints((current) => [created, ...current]);
        setSecret({ value: draft.secret ?? "whsec_demo_created", url: draft.url });
        toast.success("Webhook endpoint created in the demo");
      }
      return;
    }
    if (!store) throw new Error("No store is selected.");
    if (editing) {
      await updateWebhook(store.id, editing.id, {
        url: draft.url,
        events: draft.events,
        is_active: draft.isActive,
      });
      toast.success("Webhook endpoint updated");
    } else {
      const created = await createWebhook(store.id, {
        url: draft.url,
        events: draft.events,
        ...(draft.secret ? { secret: draft.secret } : {}),
      });
      setSecret({ value: created.secret, url: created.url });
      toast.success("Webhook endpoint created");
    }
    reload();
  };

  const confirmDelete = async () => {
    if (!deleteTarget || deleteBusy) return;
    const target = deleteTarget;
    setDeleteBusy(true);
    try {
      if (mode === "demo") {
        setDemoEndpoints((current) => current.filter((endpoint) => endpoint.id !== target.id));
      } else {
        if (!store) throw new Error("No store is selected.");
        await deleteWebhook(store.id, target.id);
      }
      setDeleteTarget(null);
      toast.success("Webhook endpoint deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDeleteBusy(false);
    }
  };

  const confirmRotate = async () => {
    if (!rotateTarget || rotateBusy) return;
    const target = rotateTarget;
    setRotateBusy(true);
    try {
      if (mode === "demo") {
        setSecret({ value: "whsec_demo_rotated", url: target.url });
      } else {
        if (!store) throw new Error("No store is selected.");
        const rotated = await rotateWebhookSecret(store.id, target.id);
        setSecret({ value: rotated.secret, url: target.url });
      }
      setRotateTarget(null);
      toast.success("Signing secret rotated");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRotateBusy(false);
    }
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Webhooks"
        description="Send signed event payloads to external services and replay deliveries that failed."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              disabled={loading}
              aria-label="Refresh webhook endpoints"
              title="Refresh webhook endpoints"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            {canWrite ? (
              <Button
                onClick={() => {
                  setEditing(null);
                  setDialogOpen(true);
                }}
              >
                <Plus className="size-4" aria-hidden="true" />
                New endpoint
              </Button>
            ) : null}
          </>
        }
      />

      <WebhookEndpointTable
        endpoints={data}
        canWrite={canWrite}
        demo={mode === "demo"}
        onDeliveries={setDeliveriesTarget}
        onEdit={(endpoint) => {
          setEditing(endpoint);
          setDialogOpen(true);
        }}
        onRotate={setRotateTarget}
        onDelete={setDeleteTarget}
      />

      <Card className="flex items-start gap-3 px-5 py-4">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-brand-ink">
          <Webhook className="size-4" aria-hidden="true" />
        </span>
        <div>
          <p className="text-[12.5px] font-medium text-foreground">Verifying signatures</p>
          <p className="mt-1 text-[11.5px] leading-relaxed text-muted-foreground">
            Every request carries an x-fastcart-signature header computed from the body and the
            endpoint secret. Rotate a secret if it may have leaked; the previous secret stops
            working immediately.
          </p>
        </div>
      </Card>

      <WebhookDialog
        open={dialogOpen}
        endpoint={editing}
        onOpenChange={(open) => {
          setDialogOpen(open);
          if (!open) setEditing(null);
        }}
        onSubmit={save}
      />

      <WebhookSecretDialog
        open={secret !== null}
        secret={secret?.value ?? null}
        endpointUrl={secret?.url ?? ""}
        onOpenChange={(open) => {
          if (!open) setSecret(null);
        }}
      />

      <WebhookDeliveriesSheet
        endpoint={deliveriesTarget}
        onOpenChange={(open) => {
          if (!open) setDeliveriesTarget(null);
        }}
      />

      <Dialog
        open={rotateTarget !== null}
        onOpenChange={(open) => {
          if (!open && !rotateBusy) setRotateTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Rotate signing secret?</DialogTitle>
            <DialogDescription>
              Deliveries to {rotateTarget?.url} will be signed with a new secret. Update your
              endpoint before the next event is sent.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRotateTarget(null)} disabled={rotateBusy}>
              Cancel
            </Button>
            <Button onClick={() => void confirmRotate()} disabled={rotateBusy}>
              {rotateBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <RotateCw className="size-4" aria-hidden="true" />
              )}
              Rotate secret
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deleteBusy) setDeleteTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete webhook endpoint?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.url}” stops receiving events. Delivery history is kept with the
              store's audit records.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteTarget(null)} disabled={deleteBusy}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void confirmDelete()}
              disabled={deleteBusy}
            >
              {deleteBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Delete endpoint
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
