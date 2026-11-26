import { History, MoreHorizontal, Pencil, RotateCw, Trash2 } from "lucide-react";
import type { WebhookEndpoint } from "@fastcart/shared-types";

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
import { eventsLabel } from "@/lib/webhooks";

/** Endpoint table with per-row actions. */
export function WebhookEndpointTable({
  endpoints,
  canWrite,
  demo,
  onDeliveries,
  onEdit,
  onRotate,
  onDelete,
}: {
  endpoints: WebhookEndpoint[];
  canWrite: boolean;
  demo: boolean;
  onDeliveries: (endpoint: WebhookEndpoint) => void;
  onEdit: (endpoint: WebhookEndpoint) => void;
  onRotate: (endpoint: WebhookEndpoint) => void;
  onDelete: (endpoint: WebhookEndpoint) => void;
}) {
  return (
    <Card className="overflow-hidden">
      <div className="flex flex-wrap items-start justify-between gap-4 px-6 pt-5 pb-2">
        <div>
          <p className="text-[13.5px] font-medium text-foreground">Endpoints</p>
          <p className="mt-1 text-[11.5px] text-muted-foreground">
            Each endpoint receives the events it subscribes to. Delivery attempts are retried with
            backoff.
          </p>
        </div>
        <p className="text-[11.5px] text-muted-foreground">
          {endpoints.length} {endpoints.length === 1 ? "endpoint" : "endpoints"}
          {demo ? " · sample data" : ""}
        </p>
      </div>

      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="pl-6">URL</TableHead>
            <TableHead className="hidden w-[240px] md:table-cell">Events</TableHead>
            <TableHead className="w-[100px]">Status</TableHead>
            <TableHead className="hidden w-[130px] lg:table-cell">Updated</TableHead>
            <TableHead className="w-10 pr-5" aria-label="Actions" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {endpoints.map((endpoint) => (
            <TableRow key={endpoint.id}>
              <TableCell className="max-w-[360px] pl-6">
                <button
                  type="button"
                  onClick={() => onDeliveries(endpoint)}
                  className="block max-w-full cursor-pointer truncate text-left font-mono text-[12px] text-foreground underline-offset-4 hover:underline"
                  title={endpoint.url}
                >
                  {endpoint.url}
                </button>
                <p className="mt-1 font-mono text-[10.5px] text-faint">{endpoint.id}</p>
              </TableCell>
              <TableCell className="hidden md:table-cell">
                <span className="text-[12px] text-ink-soft">{eventsLabel(endpoint.events)}</span>
                {endpoint.events.length > 1 ? (
                  <p className="mt-1 font-mono text-[10px] text-faint">
                    {endpoint.events.slice(1).join(", ")}
                  </p>
                ) : null}
              </TableCell>
              <TableCell>
                <Badge variant={endpoint.is_active ? "brand" : "secondary"}>
                  {endpoint.is_active ? "Active" : "Paused"}
                </Badge>
              </TableCell>
              <TableCell className="hidden text-muted-foreground lg:table-cell">
                {shortDate(fromUnixSeconds(endpoint.updated_at))}
              </TableCell>
              <TableCell className="pr-5 text-right">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={`Actions for ${endpoint.url}`}
                    >
                      <MoreHorizontal className="size-4" aria-hidden="true" />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-[200px]">
                    <DropdownMenuLabel className="truncate font-mono text-[10.5px]">
                      {endpoint.url}
                    </DropdownMenuLabel>
                    <DropdownMenuItem onSelect={() => onDeliveries(endpoint)}>
                      <History aria-hidden="true" />
                      Deliveries
                    </DropdownMenuItem>
                    {canWrite ? (
                      <DropdownMenuItem onSelect={() => onEdit(endpoint)}>
                        <Pencil aria-hidden="true" />
                        Edit endpoint
                      </DropdownMenuItem>
                    ) : null}
                    {canWrite ? (
                      <DropdownMenuItem onSelect={() => onRotate(endpoint)}>
                        <RotateCw aria-hidden="true" />
                        Rotate secret
                      </DropdownMenuItem>
                    ) : null}
                    {canWrite ? (
                      <>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem variant="destructive" onSelect={() => onDelete(endpoint)}>
                          <Trash2 aria-hidden="true" />
                          Delete
                        </DropdownMenuItem>
                      </>
                    ) : null}
                  </DropdownMenuContent>
                </DropdownMenu>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </Card>
  );
}
