import { useCallback, useEffect, useState } from "react";
import { Link2, Loader2, MoreHorizontal, Pencil, Plus, RefreshCw, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { CampaignDialog, type CampaignDraft } from "@/components/campaign-dialog";
import { CampaignPromotionsSheet } from "@/components/campaign-promotions-sheet";
import { PageHeader } from "@/components/page-header";
import { ProgressBar } from "@/components/progress-bar";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { errorMessage, fetchAll } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import {
  CAMPAIGN_BUDGET_LABELS,
  campaignBudgetFraction,
  campaignBudgetLabel,
  createCampaign,
  deleteCampaign,
  listCampaigns,
  updateCampaign,
  type Campaign,
  type UpdateCampaignInput,
} from "@/lib/campaigns";
import { demoCampaigns } from "@/lib/demo-commerce";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { PERMISSIONS } from "@/lib/permissions";
import { scheduleState, scheduleStateLabel } from "@/lib/schedule";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

export function CampaignsPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoList, setDemoList] = useState<Campaign[]>(demoCampaigns);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<Campaign | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Campaign | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [linkTarget, setLinkTarget] = useState<Campaign | null>(null);

  const canWrite = can(PERMISSIONS.promotions.write);
  const currency = store?.currency_code ?? "USD";

  const loader = useCallback(
    (signal: AbortSignal): Promise<Campaign[]> => {
      if (mode === "demo") return Promise.resolve(demoList);
      if (!store) return Promise.reject(new Error("No store is selected."));
      return fetchAll((query) => listCampaigns(store.id, query, signal));
    },
    [mode, store, demoList],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoList,
  ]);

  useEffect(() => {
    setDialogOpen(false);
    setEditing(null);
    setDeleteTarget(null);
    setLinkTarget(null);
  }, [store?.id, dataKey]);

  const save = async (draft: CampaignDraft) => {
    if (mode === "demo") {
      const timestamp = Math.floor(Date.now() / 1000);
      if (editing) {
        setDemoList((current) =>
          current.map((campaign) =>
            campaign.id === editing.id
              ? {
                  ...campaign,
                  name: draft.name,
                  description: draft.description,
                  budget_type: draft.budgetType,
                  currency_code: draft.currencyCode,
                  budget_limit: draft.budgetLimit,
                  ends_at: draft.endsAt,
                  updated_at: timestamp,
                }
              : campaign,
          ),
        );
        toast.success("Campaign updated in the demo");
      } else {
        const created: Campaign = {
          id: `camp_demo_${timestamp}`,
          name: draft.name,
          description: draft.description,
          budget_type: draft.budgetType,
          currency_code: draft.currencyCode,
          budget_limit: draft.budgetLimit,
          budget_used: 0,
          starts_at: null,
          ends_at: draft.endsAt,
          metadata: {},
          created_at: timestamp,
          updated_at: timestamp,
        };
        setDemoList((current) => [created, ...current]);
        toast.success("Campaign created in the demo");
      }
      return;
    }
    if (!store) throw new Error("No store is selected.");
    if (editing) {
      // The API distinguishes an absent field from an explicit null, so only
      // changed fields are sent. Unchanged fields are left out entirely.
      const update: UpdateCampaignInput = {};
      if (draft.name !== editing.name) update.name = draft.name;
      if (draft.description !== editing.description) update.description = draft.description;
      if (draft.budgetType !== editing.budget_type) update.budget_type = draft.budgetType;
      if (draft.currencyCode !== editing.currency_code) {
        update.currency_code = draft.currencyCode;
      }
      if (draft.budgetLimit !== editing.budget_limit) update.budget_limit = draft.budgetLimit;
      if (draft.endsAt !== editing.ends_at) update.ends_at = draft.endsAt;
      await updateCampaign(store.id, editing.id, update);
      toast.success("Campaign updated");
    } else {
      await createCampaign(store.id, {
        name: draft.name,
        description: draft.description,
        budget_type: draft.budgetType,
        currency_code: draft.currencyCode,
        budget_limit: draft.budgetLimit,
        starts_at: null,
        ends_at: draft.endsAt,
      });
      toast.success("Campaign created");
    }
    reload();
  };

  const confirmDelete = async () => {
    if (!deleteTarget || deleteBusy) return;
    const target = deleteTarget;
    setDeleteBusy(true);
    try {
      if (mode === "demo") {
        setDemoList((current) => current.filter((campaign) => campaign.id !== target.id));
      } else {
        if (!store) throw new Error("No store is selected.");
        await deleteCampaign(store.id, target.id);
      }
      setDeleteTarget(null);
      toast.success("Campaign deleted");
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
        title="Campaigns"
        description="Group promotions under a shared usage or spend budget and a single validity window."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              disabled={loading}
              aria-label="Refresh campaigns"
              title="Refresh campaigns"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            {canWrite ? (
              <Button onClick={() => setDialogOpen(true)}>
                <Plus className="size-4" aria-hidden="true" />
                New campaign
              </Button>
            ) : null}
          </>
        }
      />

      <Card className="overflow-hidden">
        <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">All campaigns</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              Budgets are consumed by the promotions linked to each campaign.
            </p>
          </div>
          <p className="text-[11.5px] text-muted-foreground">
            {data.length} {data.length === 1 ? "campaign" : "campaigns"}
            {mode === "demo" ? " · sample data" : ""}
          </p>
        </div>

        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Campaign</TableHead>
              <TableHead className="hidden w-[110px] md:table-cell">Budget</TableHead>
              <TableHead className="w-[190px]">Consumed</TableHead>
              <TableHead className="hidden w-[120px] lg:table-cell">Window</TableHead>
              <TableHead className="hidden w-[130px] xl:table-cell">Ends</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.map((campaign) => {
              const fraction = campaignBudgetFraction(campaign.budget_used, campaign.budget_limit);
              const state = scheduleState(
                campaign.starts_at,
                campaign.ends_at,
                Math.floor(Date.now() / 1000),
              );
              return (
                <TableRow key={campaign.id}>
                  <TableCell className="pl-6">
                    <button
                      type="button"
                      onClick={() => setLinkTarget(campaign)}
                      className="cursor-pointer text-left text-[13px] font-medium text-foreground underline-offset-4 hover:underline"
                    >
                      {campaign.name}
                    </button>
                    <p className="mt-1 font-mono text-[10.5px] text-faint">{campaign.id}</p>
                  </TableCell>
                  <TableCell className="hidden md:table-cell">
                    <Badge variant={campaign.budget_type === "spend" ? "info" : "secondary"}>
                      {CAMPAIGN_BUDGET_LABELS[campaign.budget_type]}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <p className="text-[11.5px] text-ink-soft tnum">
                      {campaignBudgetLabel(campaign)}
                    </p>
                    {fraction !== null ? (
                      <ProgressBar
                        className="mt-1.5"
                        value={campaign.budget_used}
                        max={campaign.budget_limit ?? 0}
                        tone={fraction >= 1 ? "destructive" : "brand"}
                      />
                    ) : null}
                  </TableCell>
                  <TableCell className="hidden text-[12px] text-ink-soft lg:table-cell">
                    {scheduleStateLabel(state)}
                  </TableCell>
                  <TableCell className="hidden text-muted-foreground xl:table-cell">
                    {campaign.ends_at === null
                      ? "No end date"
                      : shortDate(fromUnixSeconds(campaign.ends_at))}
                  </TableCell>
                  <TableCell className="pr-5 text-right">
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Actions for ${campaign.name}`}
                        >
                          <MoreHorizontal className="size-4" aria-hidden="true" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-[190px]">
                        <DropdownMenuLabel className="truncate">{campaign.name}</DropdownMenuLabel>
                        <DropdownMenuItem onSelect={() => setLinkTarget(campaign)}>
                          <Link2 aria-hidden="true" />
                          Manage promotions
                        </DropdownMenuItem>
                        {canWrite ? (
                          <DropdownMenuItem
                            onSelect={() => {
                              setEditing(campaign);
                              setDialogOpen(true);
                            }}
                          >
                            <Pencil aria-hidden="true" />
                            Edit campaign
                          </DropdownMenuItem>
                        ) : null}
                        {canWrite ? (
                          <>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              variant="destructive"
                              onSelect={() => setDeleteTarget(campaign)}
                            >
                              <Trash2 aria-hidden="true" />
                              Delete
                            </DropdownMenuItem>
                          </>
                        ) : null}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              );
            })}
            {data.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={6} className="p-0">
                  <EmptyState
                    title="No campaigns"
                    description="Create a campaign to cap how much a group of promotions can give away."
                    action={
                      canWrite ? (
                        <Button onClick={() => setDialogOpen(true)}>
                          <Plus className="size-4" aria-hidden="true" />
                          New campaign
                        </Button>
                      ) : null
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </Card>

      <CampaignDialog
        open={dialogOpen}
        campaign={editing}
        defaultCurrency={currency}
        onOpenChange={(open) => {
          setDialogOpen(open);
          if (!open) setEditing(null);
        }}
        onSubmit={save}
      />

      <CampaignPromotionsSheet
        campaign={linkTarget}
        onOpenChange={(open) => {
          if (!open) setLinkTarget(null);
        }}
        onChanged={() => {
          if (mode !== "demo") reload();
        }}
      />

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deleteBusy) setDeleteTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete campaign?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.name}” will be removed and its promotion links released. The
              promotions themselves are kept.
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
              Delete campaign
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
