import { useEffect, useState } from "react";
import { ChevronDown, Loader2 } from "lucide-react";
import { toast } from "sonner";
import type { PriceList, PriceListStatus, PriceListType } from "@fastcart/shared-types";

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
  PRICE_LIST_STATUSES,
  PRICE_LIST_STATUS_LABELS,
  PRICE_LIST_TYPE_LABELS,
  PRICE_LIST_TYPES,
  priceListTypeHint,
} from "@/lib/price-lists";
import { dateInputToUnixSeconds, unixSecondsToDateInput } from "@/lib/schedule";

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card pr-9 pl-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:cursor-not-allowed disabled:opacity-70";

export interface PriceListDraft {
  title: string;
  description: string | null;
  priceType: PriceListType;
  status: PriceListStatus;
  endsAt: number | null;
}

export function PriceListDialog({
  open,
  priceList,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  priceList: PriceList | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (draft: PriceListDraft) => Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [priceType, setPriceType] = useState<PriceListType>("sale");
  const [status, setStatus] = useState<PriceListStatus>("draft");
  const [endsAt, setEndsAt] = useState("");
  const [titleError, setTitleError] = useState<string | null>(null);
  const [endsError, setEndsError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setTitle(priceList?.title ?? "");
    setDescription(priceList?.description ?? "");
    setPriceType(priceList?.price_type ?? "sale");
    setStatus(priceList?.status ?? "draft");
    setEndsAt(unixSecondsToDateInput(priceList?.ends_at ?? null));
    setTitleError(null);
    setEndsError(null);
    setBusy(false);
  }, [open, priceList?.id]);

  const submit = async () => {
    if (busy) return;
    const trimmed = title.trim();
    if (trimmed.length === 0) {
      setTitleError("Enter a name for this price list.");
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
        title: trimmed,
        description: description.trim().length === 0 ? null : description.trim(),
        priceType,
        status,
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
            <DialogTitle>{priceList ? "Edit price list" : "New price list"}</DialogTitle>
            <DialogDescription>
              A price list groups variant prices that are applied together, such as a seasonal sale
              or a wholesale override.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-2">
            <Label htmlFor="price-list-title">Name</Label>
            <Input
              id="price-list-title"
              value={title}
              onChange={(event) => {
                setTitle(event.target.value);
                setTitleError(null);
              }}
              placeholder="Wholesale spring 2027"
              autoComplete="off"
              disabled={busy}
              aria-invalid={titleError !== null}
            />
            {titleError ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {titleError}
              </p>
            ) : null}
          </div>

          <div className="space-y-2">
            <Label htmlFor="price-list-description">Description</Label>
            <textarea
              id="price-list-description"
              rows={2}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              disabled={busy}
              placeholder="Applied to trade accounts only"
              className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs transition-[color,box-shadow,border-color] outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
            />
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="price-list-type">Type</Label>
              <div className="relative">
                <select
                  id="price-list-type"
                  value={priceType}
                  onChange={(event) => setPriceType(event.target.value as PriceListType)}
                  disabled={busy}
                  className={selectClass}
                >
                  {PRICE_LIST_TYPES.map((option) => (
                    <option key={option} value={option}>
                      {PRICE_LIST_TYPE_LABELS[option]}
                    </option>
                  ))}
                </select>
                <ChevronDown
                  className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                  aria-hidden="true"
                />
              </div>
              <p className="text-[11px] text-faint">{priceListTypeHint(priceType)}</p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="price-list-status">Status</Label>
              <div className="relative">
                <select
                  id="price-list-status"
                  value={status}
                  onChange={(event) => setStatus(event.target.value as PriceListStatus)}
                  disabled={busy}
                  className={selectClass}
                >
                  {PRICE_LIST_STATUSES.map((option) => (
                    <option key={option} value={option}>
                      {PRICE_LIST_STATUS_LABELS[option]}
                    </option>
                  ))}
                </select>
                <ChevronDown
                  className="pointer-events-none absolute top-1/2 right-3 size-3.5 -translate-y-1/2 text-faint"
                  aria-hidden="true"
                />
              </div>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="price-list-ends">Ends</Label>
            <Input
              id="price-list-ends"
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
              Leave empty for a list with no end date. Draft lists never apply.
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
              {priceList ? "Save changes" : "Create price list"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
