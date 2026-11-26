import { useEffect, useState } from "react";
import { ChevronDown, Loader2 } from "lucide-react";
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
import {
  CAMPAIGN_BUDGET_LABELS,
  CAMPAIGN_BUDGET_TYPES,
  campaignBudgetProblem,
  type Campaign,
  type CampaignBudgetType,
} from "@/lib/campaigns";
import {
  formatMinorInput,
  isValidCurrency,
  normalizeCurrency,
  parsePriceAmount,
} from "@/lib/pricing";
import { dateInputToUnixSeconds, unixSecondsToDateInput } from "@/lib/schedule";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70";

export interface CampaignDraft {
  name: string;
  description: string | null;
  budgetType: CampaignBudgetType;
  currencyCode: string | null;
  budgetLimit: number | null;
  endsAt: number | null;
}

export function CampaignDialog({
  open,
  campaign,
  defaultCurrency,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  campaign: Campaign | null;
  defaultCurrency: string;
  onOpenChange: (open: boolean) => void;
  onSubmit: (draft: CampaignDraft) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [budgetType, setBudgetType] = useState<CampaignBudgetType>("usage");
  const [currency, setCurrency] = useState(defaultCurrency);
  const [budget, setBudget] = useState("");
  const [endsAt, setEndsAt] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);
  const [budgetError, setBudgetError] = useState<string | null>(null);
  const [currencyError, setCurrencyError] = useState<string | null>(null);
  const [endsError, setEndsError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setBudgetType(campaign?.budget_type ?? "usage");
    setName(campaign?.name ?? "");
    setDescription(campaign?.description ?? "");
    setCurrency(campaign?.currency_code ?? defaultCurrency);
    setBudget(
      campaign === null
        ? ""
        : campaign.budget_type === "spend" && campaign.budget_limit !== null
          ? formatMinorInput(campaign.budget_limit)
          : (campaign.budget_limit?.toString() ?? ""),
    );
    setEndsAt(unixSecondsToDateInput(campaign?.ends_at ?? null));
    setNameError(null);
    setBudgetError(null);
    setCurrencyError(null);
    setEndsError(null);
    setBusy(false);
  }, [open, campaign?.id, defaultCurrency]);

  const submit = async () => {
    if (busy) return;
    const trimmedName = name.trim();
    if (trimmedName.length === 0) {
      setNameError("Enter a name for this campaign.");
      return;
    }

    let budgetLimit: number | null = null;
    if (budget.trim().length > 0) {
      if (budgetType === "spend") {
        const parsed = parsePriceAmount(budget);
        if (!parsed.ok) {
          setBudgetError(parsed.message);
          return;
        }
        budgetLimit = parsed.minor;
      } else {
        if (!/^\d+$/.test(budget.trim())) {
          setBudgetError("Enter a whole number of uses, or leave it empty for unlimited.");
          return;
        }
        budgetLimit = Number(budget.trim());
      }
    }

    let currencyCode: string | null = null;
    if (budgetType === "spend") {
      if (!isValidCurrency(currency)) {
        setCurrencyError("Enter a three-letter ISO currency code.");
        return;
      }
      currencyCode = normalizeCurrency(currency);
    }

    const problem = campaignBudgetProblem(budgetType, currencyCode, budgetLimit);
    if (problem) {
      if (budgetType === "spend" && currencyCode === null) setCurrencyError(problem);
      else setBudgetError(problem);
      return;
    }

    let endsAtSeconds: number | null = null;
    if (endsAt.trim().length > 0) {
      endsAtSeconds = dateInputToUnixSeconds(endsAt);
      if (endsAtSeconds === null) {
        setEndsError("Enter a valid end date.");
        return;
      }
    }

    setBusy(true);
    try {
      await onSubmit({
        name: trimmedName,
        description: description.trim().length === 0 ? null : description.trim(),
        budgetType,
        currencyCode,
        budgetLimit,
        endsAt: endsAtSeconds,
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
            <DialogTitle>{campaign ? "Edit campaign" : "New campaign"}</DialogTitle>
            <DialogDescription>
              A campaign shares one budget across the promotions linked to it.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="campaign-name">Name</Label>
            <Input
              id="campaign-name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setNameError(null);
              }}
              placeholder="Autumn studio edit"
              autoComplete="off"
              disabled={busy}
              aria-invalid={nameError !== null}
            />
            {nameError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {nameError}
              </p>
            ) : null}
          </div>

          <div className="space-y-2">
            <Label htmlFor="campaign-description">Description</Label>
            <textarea
              id="campaign-description"
              rows={2}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              disabled={busy}
              placeholder="Seasonal promotions sharing a quarterly budget"
              className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
            />
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="campaign-budget-type">Budget type</Label>
              <div className="relative">
                <select
                  id="campaign-budget-type"
                  value={budgetType}
                  onChange={(event) => {
                    setBudgetType(event.target.value as CampaignBudgetType);
                    setBudgetError(null);
                    setCurrencyError(null);
                  }}
                  disabled={busy}
                  className={selectClass}
                >
                  {CAMPAIGN_BUDGET_TYPES.map((option) => (
                    <option key={option} value={option}>
                      {CAMPAIGN_BUDGET_LABELS[option]}
                    </option>
                  ))}
                </select>
                <ChevronDown
                  className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                  aria-hidden="true"
                />
              </div>
            </div>
            <div className="space-y-2">
              <Label htmlFor="campaign-budget">
                {budgetType === "spend" ? "Budget amount" : "Max uses"}
              </Label>
              <Input
                id="campaign-budget"
                type="number"
                min="0"
                step={budgetType === "spend" ? "0.01" : "1"}
                inputMode={budgetType === "spend" ? "decimal" : "numeric"}
                value={budget}
                onChange={(event) => {
                  setBudget(event.target.value);
                  setBudgetError(null);
                }}
                placeholder={budgetType === "spend" ? "2,500.00" : "Unlimited"}
                disabled={busy}
                aria-invalid={budgetError !== null}
              />
              {budgetError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {budgetError}
                </p>
              ) : null}
            </div>
          </div>

          {budgetType === "spend" ? (
            <div className="space-y-2">
              <Label htmlFor="campaign-currency">Currency</Label>
              <Input
                id="campaign-currency"
                value={currency}
                onChange={(event) => {
                  setCurrency(event.target.value.toUpperCase().slice(0, 3));
                  setCurrencyError(null);
                }}
                maxLength={3}
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                aria-invalid={currencyError !== null}
                className="font-mono uppercase"
              />
              {currencyError ? (
                <p className="text-[11.5px] text-destructive" role="alert">
                  {currencyError}
                </p>
              ) : null}
            </div>
          ) : null}

          <div className="space-y-2">
            <Label htmlFor="campaign-ends">Ends</Label>
            <Input
              id="campaign-ends"
              type="date"
              value={endsAt}
              onChange={(event) => {
                setEndsAt(event.target.value);
                setEndsError(null);
              }}
              disabled={busy}
              aria-invalid={endsError !== null}
            />
            <p className="text-[11px] text-faint">
              Leave empty for a campaign with no end date. Changing the budget type or currency is
              rejected once the campaign has recorded usage.
            </p>
            {endsError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {endsError}
              </p>
            ) : null}
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
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {campaign ? "Save changes" : "Create campaign"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
