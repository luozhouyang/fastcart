import { Fragment } from "react";
import { ChevronDown, ChevronRight, MoreHorizontal, Pencil, Trash2 } from "lucide-react";
import type { PriceList } from "@fastcart/shared-types";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
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
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  PRICE_LIST_STATUS_LABELS,
  PRICE_LIST_TYPE_LABELS,
  type PriceListPrice,
} from "@/lib/price-lists";
import { formatMinorAmount } from "@/lib/pricing";
import { scheduleState, scheduleStateLabel } from "@/lib/schedule";

export interface PriceListExpandState {
  expanded: Record<string, boolean>;
  data: Record<string, PriceListPrice[]>;
  loading: Record<string, boolean>;
  error: Record<string, string | null>;
}

/** Read-only table of price lists with an expandable price preview. */
export function PriceListTable({
  lists,
  expand,
  canWrite,
  demo,
  onToggle,
  onEdit,
  onDelete,
}: {
  lists: PriceList[];
  expand: PriceListExpandState;
  canWrite: boolean;
  demo: boolean;
  onToggle: (list: PriceList) => void;
  onEdit: (list: PriceList) => void;
  onDelete: (list: PriceList) => void;
}) {
  const now = Math.floor(Date.now() / 1000);

  return (
    <Card className="overflow-hidden">
      <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
        <div>
          <p className="text-[13.5px] font-medium text-foreground">All price lists</p>
          <p className="mt-1 text-[11.5px] text-muted-foreground">
            Expand a list to inspect the prices it holds. Draft lists never apply at checkout.
          </p>
        </div>
        <p className="text-[11.5px] text-muted-foreground">
          {lists.length} {lists.length === 1 ? "list" : "lists"}
          {demo ? " · sample data" : ""}
        </p>
      </div>

      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="w-9 pl-6" aria-label="Expand" />
            <TableHead>Name</TableHead>
            <TableHead className="w-[110px]">Status</TableHead>
            <TableHead className="w-[110px]">Type</TableHead>
            <TableHead className="hidden w-[140px] md:table-cell">Window</TableHead>
            <TableHead className="hidden w-[130px] lg:table-cell">Ends</TableHead>
            <TableHead className="w-10 pr-5" aria-label="Actions" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {lists.map((list) => {
            const isOpen = expand.expanded[list.id] === true;
            const prices = expand.data[list.id] ?? [];
            return (
              <Fragment key={list.id}>
                <TableRow>
                  <TableCell className="pl-6">
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={() => onToggle(list)}
                      aria-expanded={isOpen}
                      aria-label={`${isOpen ? "Collapse" : "Expand"} ${list.title}`}
                    >
                      {isOpen ? (
                        <ChevronDown className="size-4" aria-hidden="true" />
                      ) : (
                        <ChevronRight className="size-4" aria-hidden="true" />
                      )}
                    </Button>
                  </TableCell>
                  <TableCell>
                    <span className="text-[13px] font-medium text-foreground">{list.title}</span>
                    <p className="mt-1 font-mono text-[10.5px] text-faint">{list.id}</p>
                  </TableCell>
                  <TableCell>
                    <Badge variant={list.status === "active" ? "brand" : "secondary"}>
                      {PRICE_LIST_STATUS_LABELS[list.status]}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <Badge variant="outline">{PRICE_LIST_TYPE_LABELS[list.price_type]}</Badge>
                  </TableCell>
                  <TableCell className="hidden text-[12px] text-ink-soft md:table-cell">
                    {scheduleStateLabel(scheduleState(list.starts_at, list.ends_at, now))}
                  </TableCell>
                  <TableCell className="hidden text-muted-foreground lg:table-cell">
                    {list.ends_at === null
                      ? "No end date"
                      : shortDate(fromUnixSeconds(list.ends_at))}
                  </TableCell>
                  <TableCell className="pr-5 text-right">
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Actions for ${list.title}`}
                        >
                          <MoreHorizontal className="size-4" aria-hidden="true" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-[190px]">
                        <DropdownMenuLabel className="truncate">{list.title}</DropdownMenuLabel>
                        <DropdownMenuItem onSelect={() => onToggle(list)}>
                          <ChevronRight aria-hidden="true" />
                          {isOpen ? "Hide prices" : "Show prices"}
                        </DropdownMenuItem>
                        {canWrite ? (
                          <DropdownMenuItem onSelect={() => onEdit(list)}>
                            <Pencil aria-hidden="true" />
                            Edit price list
                          </DropdownMenuItem>
                        ) : null}
                        {canWrite ? (
                          <>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem variant="destructive" onSelect={() => onDelete(list)}>
                              <Trash2 aria-hidden="true" />
                              Delete
                            </DropdownMenuItem>
                          </>
                        ) : null}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
                {isOpen ? (
                  <TableRow className="hover:bg-transparent">
                    <TableCell colSpan={7} className="bg-secondary/30 p-4 pl-14">
                      <PricePreview
                        loading={expand.loading[list.id] === true}
                        error={expand.error[list.id] ?? null}
                        prices={prices}
                      />
                    </TableCell>
                  </TableRow>
                ) : null}
              </Fragment>
            );
          })}
        </TableBody>
      </Table>
    </Card>
  );
}

function PricePreview({
  loading,
  error,
  prices,
}: {
  loading: boolean;
  error: string | null;
  prices: PriceListPrice[];
}) {
  if (loading) return <p className="text-[12px] text-muted-foreground">Loading prices…</p>;
  if (error) return <p className="text-[12px] text-destructive">{error}</p>;
  if (prices.length === 0) {
    return (
      <p className="text-[12px] text-muted-foreground">
        No prices are attached to this list yet. Variant prices can be assigned to it from the
        product editor.
      </p>
    );
  }
  return (
    <Table>
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <TableHead>Variant</TableHead>
          <TableHead className="w-[130px]">Amount</TableHead>
          <TableHead className="w-[110px]">Min qty</TableHead>
          <TableHead className="hidden w-[110px] md:table-cell">Max qty</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {prices.map((price) => (
          <TableRow key={price.id}>
            <TableCell className="font-mono text-[11px] text-ink-soft">
              {price.variant_id}
            </TableCell>
            <TableCell className="tnum">
              {formatMinorAmount(price.amount, price.currency_code)}
            </TableCell>
            <TableCell className="text-[12px] tnum">{price.min_quantity}</TableCell>
            <TableCell className="hidden text-[12px] tnum md:table-cell">
              {price.max_quantity ?? "—"}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
