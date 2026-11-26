import { useCallback, useEffect, useState } from "react";
import { Loader2, Plus, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { PriceList } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { PriceListDialog, type PriceListDraft } from "@/components/price-list-dialog";
import { PriceListTable } from "@/components/price-list-table";
import { ErrorState, TableSkeleton } from "@/components/states";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorMessage, fetchAll } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { demoPriceListPrices, demoPriceLists } from "@/lib/demo-commerce";
import {
  createPriceList,
  deletePriceList,
  listPriceListPrices,
  listPriceLists,
  updatePriceList,
  type PriceListPrice,
} from "@/lib/price-lists";
import { PERMISSIONS } from "@/lib/permissions";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const now = () => Math.floor(Date.now() / 1000);

export function PriceListsPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoLists, setDemoLists] = useState<PriceList[]>(demoPriceLists);
  const [demoPrices, setDemoPrices] =
    useState<Record<string, PriceListPrice[]>>(demoPriceListPrices);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<PriceList | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<PriceList | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [expandedData, setExpandedData] = useState<Record<string, PriceListPrice[]>>({});
  const [expandedLoading, setExpandedLoading] = useState<Record<string, boolean>>({});
  const [expandedError, setExpandedError] = useState<Record<string, string | null>>({});

  const canWrite = can(PERMISSIONS.promotions.write);
  const currency = store?.currency_code ?? "USD";

  const loader = useCallback(
    (signal: AbortSignal): Promise<PriceList[]> => {
      if (mode === "demo") return Promise.resolve(demoLists);
      if (!store) return Promise.reject(new Error("No store is selected."));
      return fetchAll((query) => listPriceLists(store.id, query, signal));
    },
    [mode, store, demoLists],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoLists,
  ]);

  useEffect(() => {
    setDialogOpen(false);
    setEditing(null);
    setDeleteTarget(null);
    setExpanded({});
    setExpandedData({});
    setExpandedError({});
  }, [store?.id, dataKey]);

  const openCreate = () => {
    setEditing(null);
    setDialogOpen(true);
  };

  const openEdit = (priceList: PriceList) => {
    setEditing(priceList);
    setDialogOpen(true);
  };

  const loadPrices = useCallback(
    async (priceListId: string) => {
      if (mode === "demo") {
        setExpandedData((current) => ({
          ...current,
          [priceListId]: demoPrices[priceListId] ?? [],
        }));
        return;
      }
      if (!store) return;
      setExpandedLoading((current) => ({ ...current, [priceListId]: true }));
      setExpandedError((current) => ({ ...current, [priceListId]: null }));
      try {
        const prices = await fetchAll((query) => listPriceListPrices(store.id, priceListId, query));
        setExpandedData((current) => ({ ...current, [priceListId]: prices }));
      } catch (cause) {
        setExpandedError((current) => ({ ...current, [priceListId]: errorMessage(cause) }));
      } finally {
        setExpandedLoading((current) => ({ ...current, [priceListId]: false }));
      }
    },
    [mode, store, demoPrices],
  );

  const toggleExpanded = (priceList: PriceList) => {
    const next = !expanded[priceList.id];
    setExpanded((current) => ({ ...current, [priceList.id]: next }));
    if (next && expandedData[priceList.id] === undefined) void loadPrices(priceList.id);
  };

  const save = async (draft: PriceListDraft) => {
    if (mode === "demo") {
      const timestamp = now();
      if (editing) {
        setDemoLists((current) =>
          current.map((entry) =>
            entry.id === editing.id
              ? {
                  ...entry,
                  title: draft.title,
                  description: draft.description,
                  price_type: draft.priceType,
                  status: draft.status,
                  ends_at: draft.endsAt,
                  updated_at: timestamp,
                }
              : entry,
          ),
        );
        toast.success("Price list updated in the demo");
      } else {
        const created: PriceList = {
          id: `pl_demo_${timestamp}`,
          title: draft.title,
          description: draft.description,
          price_type: draft.priceType,
          status: draft.status,
          starts_at: null,
          ends_at: draft.endsAt,
          metadata: {},
          rules: [],
          created_at: timestamp,
          updated_at: timestamp,
        };
        setDemoLists((current) => [created, ...current]);
        setDemoPrices((current) => ({ ...current, [created.id]: [] }));
        toast.success("Price list created in the demo");
      }
      return;
    }
    if (!store) throw new Error("No store is selected.");
    if (editing) {
      await updatePriceList(store.id, editing.id, {
        title: draft.title,
        description: draft.description,
        price_type: draft.priceType,
        status: draft.status,
        ends_at: draft.endsAt,
      });
      toast.success("Price list updated");
    } else {
      await createPriceList(store.id, {
        title: draft.title,
        description: draft.description,
        price_type: draft.priceType,
        status: draft.status,
        ends_at: draft.endsAt,
      });
      toast.success("Price list created");
    }
    reload();
  };

  const confirmDelete = async () => {
    if (!deleteTarget || deleteBusy) return;
    const target = deleteTarget;
    setDeleteBusy(true);
    try {
      if (mode === "demo") {
        setDemoLists((current) => current.filter((entry) => entry.id !== target.id));
        setDemoPrices((current) => {
          const next = { ...current };
          delete next[target.id];
          return next;
        });
      } else {
        if (!store) throw new Error("No store is selected.");
        await deletePriceList(store.id, target.id);
      }
      setDeleteTarget(null);
      toast.success("Price list deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDeleteBusy(false);
    }
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Price lists"
        description="Group variant prices into periodic sales or absolute overrides that apply together."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              disabled={loading}
              aria-label="Refresh price lists"
              title="Refresh price lists"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            {canWrite ? (
              <Button onClick={openCreate}>
                <Plus className="size-4" aria-hidden="true" />
                New price list
              </Button>
            ) : null}
          </>
        }
      />

      <PriceListTable
        lists={data}
        expand={{
          expanded,
          data: expandedData,
          loading: expandedLoading,
          error: expandedError,
        }}
        canWrite={canWrite}
        demo={mode === "demo"}
        onToggle={toggleExpanded}
        onEdit={openEdit}
        onDelete={setDeleteTarget}
      />

      <PriceListDialog
        open={dialogOpen}
        priceList={editing}
        onOpenChange={(open) => {
          setDialogOpen(open);
          if (!open) setEditing(null);
        }}
        onSubmit={save}
      />

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deleteBusy) setDeleteTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete price list?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.title}” will be removed. Prices attached to it may stop applying; the
              API error is shown if any price still references it.
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
              Delete price list
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <p className="text-[10.5px] text-faint">
        Price amounts are stored in minor units of {currency}. A list that has ended no longer
        applies even when its status is active.
      </p>
    </div>
  );
}
