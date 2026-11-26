import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, Link2, Loader2, Unlink } from "lucide-react";
import { toast } from "sonner";

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
import {
  linkablePromotions,
  listCampaignPromotions,
  linkCampaignPromotion,
  unlinkCampaignPromotion,
  type Campaign,
} from "@/lib/campaigns";
import { demoCampaignLinks, demoCampaignPromotionChoices } from "@/lib/demo-commerce";
import { PERMISSIONS } from "@/lib/permissions";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70";

interface PromotionChoice {
  id: string;
  title: string;
}

/**
 * Shows and edits the promotions linked to one campaign. The campaign table
 * only needs the list of ids, so titles are resolved from the store's
 * promotions when available.
 */
export function CampaignPromotionsSheet({
  campaign,
  onOpenChange,
  onChanged,
}: {
  campaign: Campaign | null;
  onOpenChange: (open: boolean) => void;
  onChanged: () => void;
}) {
  const { mode, client, store, can } = useAuth();
  const [demoLinks, setDemoLinks] = useState<Record<string, string[]>>(demoCampaignLinks);
  const [choices, setChoices] = useState<PromotionChoice[]>([]);
  const [linkedIds, setLinkedIds] = useState<string[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const campaignId = campaign?.id ?? null;
  const canWrite = can(PERMISSIONS.promotions.write);

  useEffect(() => {
    if (campaignId === null) return;
    setLoadError(null);
    setSelectedId("");

    if (mode === "demo") {
      setChoices(demoCampaignPromotionChoices());
      setLinkedIds(demoLinks[campaignId] ?? []);
      return;
    }

    if (!client || !store) return;
    const storeId = store.id;
    const controller = new AbortController();
    setLoading(true);
    void Promise.all([
      client.promotions.list({ limit: 200 }, { signal: controller.signal }),
      listCampaignPromotions(storeId, campaignId, controller.signal),
    ])
      .then(([page, ids]) => {
        if (controller.signal.aborted) return;
        setChoices(page.data.map((promotion) => ({ id: promotion.id, title: promotion.title })));
        setLinkedIds(ids);
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        setLoadError(errorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [campaignId, mode, client, store, demoLinks]);

  const titleFor = useCallback(
    (promotionId: string) =>
      choices.find((choice) => choice.id === promotionId)?.title ?? promotionId,
    [choices],
  );

  const available = useMemo(() => linkablePromotions(choices, linkedIds), [choices, linkedIds]);

  const link = async () => {
    if (campaignId === null || selectedId.length === 0 || busyId !== null) return;
    const promotionId = selectedId;
    setBusyId(promotionId);
    try {
      if (mode === "demo") {
        setDemoLinks((current) => ({
          ...current,
          [campaignId]: [...(current[campaignId] ?? []), promotionId],
        }));
        setLinkedIds((current) => [...current, promotionId]);
      } else if (store) {
        await linkCampaignPromotion(store.id, campaignId, promotionId);
        setLinkedIds((current) => [...current, promotionId]);
      }
      setSelectedId("");
      toast.success("Promotion linked");
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  };

  const unlink = async (promotionId: string) => {
    if (campaignId === null || busyId !== null) return;
    setBusyId(promotionId);
    try {
      if (mode === "demo") {
        setDemoLinks((current) => ({
          ...current,
          [campaignId]: (current[campaignId] ?? []).filter((id) => id !== promotionId),
        }));
        setLinkedIds((current) => current.filter((id) => id !== promotionId));
      } else if (store) {
        await unlinkCampaignPromotion(store.id, campaignId, promotionId);
        setLinkedIds((current) => current.filter((id) => id !== promotionId));
      }
      toast.success("Promotion unlinked");
      onChanged();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <Sheet open={campaign !== null} onOpenChange={onOpenChange}>
      <SheetContent className="flex flex-col sm:max-w-lg">
        <SheetHeader>
          <SheetTitle>{campaign?.name} promotions</SheetTitle>
          <SheetDescription>
            Promotions linked here share the campaign budget. A promotion can only belong to one
            campaign at a time.
          </SheetDescription>
        </SheetHeader>

        <div className="flex-1 space-y-5 overflow-y-auto p-6">
          {loadError ? (
            <p className="rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-[12px] text-destructive">
              {loadError}
            </p>
          ) : null}

          <section>
            <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
              Linked promotions
            </p>
            {loading ? (
              <div className="flex justify-center py-6">
                <Loader2 className="size-4 animate-spin text-faint" aria-hidden="true" />
              </div>
            ) : linkedIds.length === 0 ? (
              <p className="mt-2 rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12px] text-muted-foreground">
                No promotions are linked to this campaign.
              </p>
            ) : (
              <ul className="mt-2 divide-y divide-border overflow-hidden rounded-lg border border-border">
                {linkedIds.map((promotionId) => (
                  <li
                    key={promotionId}
                    className="flex items-center justify-between gap-3 bg-background px-3 py-2.5"
                  >
                    <span className="min-w-0">
                      <span className="block truncate text-[12.5px] text-foreground">
                        {titleFor(promotionId)}
                      </span>
                      <span className="mt-0.5 block font-mono text-[10px] text-faint">
                        {promotionId}
                      </span>
                    </span>
                    {canWrite ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => void unlink(promotionId)}
                        disabled={busyId !== null}
                      >
                        {busyId === promotionId ? (
                          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                        ) : (
                          <Unlink className="size-3.5" aria-hidden="true" />
                        )}
                        Unlink
                      </Button>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </section>

          {canWrite ? (
            <section className="space-y-2">
              <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
                Link a promotion
              </p>
              <div className="flex gap-2">
                <div className="relative min-w-0 flex-1">
                  <select
                    aria-label="Promotion to link"
                    value={selectedId}
                    onChange={(event) => setSelectedId(event.target.value)}
                    disabled={busyId !== null || available.length === 0}
                    className={selectClass}
                  >
                    <option value="">
                      {available.length === 0 ? "No promotions available" : "Select a promotion"}
                    </option>
                    {available.map((promotion) => (
                      <option key={promotion.id} value={promotion.id}>
                        {promotion.title}
                      </option>
                    ))}
                  </select>
                  <ChevronDown
                    className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                    aria-hidden="true"
                  />
                </div>
                <Button
                  onClick={() => void link()}
                  disabled={selectedId.length === 0 || busyId !== null}
                >
                  {busyId !== null && busyId === selectedId ? (
                    <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                  ) : (
                    <Link2 className="size-4" aria-hidden="true" />
                  )}
                  Link
                </Button>
              </div>
            </section>
          ) : (
            <p className="text-[11px] text-faint">
              Linking promotions requires the promotions.write permission.
            </p>
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
