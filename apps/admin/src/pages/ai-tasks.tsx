import { useCallback, useEffect, useState } from "react";
import { Ban, ChevronDown, ChevronRight, Loader2, Play, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import type { AiTask, AiTool } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { Pagination } from "@/components/pagination";
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
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  aiTaskFilterOptions,
  aiTaskStatusLabel,
  aiTaskStatusTone,
  approveAiTask,
  listAiTasks,
  listAiTools,
  rejectAiTask,
  type AiTaskStatus,
} from "@/lib/ai-tasks";
import { errorMessage, fetchAll } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { PERMISSIONS } from "@/lib/permissions";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 20;

/** Fixed demo clock so seeded timestamps stay stable between renders. */
const DEMO_CLOCK = Math.floor(new Date("2026-11-26T12:00:00Z").getTime() / 1000);
const DAY = 86_400;

function daysAgo(days: number): number {
  return DEMO_CLOCK - days * DAY;
}

type StatusFilter = AiTaskStatus | "all";

interface AiTasksData {
  tools: AiTool[];
  tasks: AiTask[];
  total: number;
}

/**
 * Operator console for the AI tool layer.
 *
 * The API only records invocations and their approvals; no model runs inside
 * the admin. Approving a pending task lets it run, so the confirmation step is
 * deliberate.
 */
export function AiTasksPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoTools] = useState<AiTool[]>(demoAiTools);
  const [demoTasks, setDemoTasks] = useState<AiTask[]>(demoAiTasks);
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [page, setPage] = useState(1);
  const [detail, setDetail] = useState<AiTask | null>(null);
  const [confirming, setConfirming] = useState<AiTask | null>(null);
  const [actionBusy, setActionBusy] = useState(false);

  const canRead = can(PERMISSIONS.settings.read);
  const canWrite = can(PERMISSIONS.settings.write);

  useEffect(() => {
    setPage(1);
    setDetail(null);
    setConfirming(null);
  }, [store?.id, dataKey, mode]);

  const loader = useCallback(
    async (signal: AbortSignal): Promise<AiTasksData> => {
      if (!canRead) {
        throw new Error("Your account does not have the settings.read permission.");
      }
      const offset = (page - 1) * PAGE_SIZE;
      if (mode === "demo") {
        const filtered =
          statusFilter === "all"
            ? demoTasks
            : demoTasks.filter((task) => task.status === statusFilter);
        const sorted = [...filtered].sort((left, right) => right.created_at - left.created_at);
        return {
          tools: demoTools,
          tasks: sorted.slice(offset, offset + PAGE_SIZE),
          total: sorted.length,
        };
      }
      if (!store) throw new Error("No store is selected.");
      if (statusFilter === "all") {
        const [tools, tasks] = await Promise.all([
          listAiTools(store.id, signal),
          listAiTasks(store.id, { limit: PAGE_SIZE, offset }, signal),
        ]);
        return { tools, tasks: tasks.data, total: tasks.pagination.total };
      }
      // The list endpoint has no status filter, so a filtered view reads every
      // page and applies the filter before paginating locally.
      const [tools, all] = await Promise.all([
        listAiTools(store.id, signal),
        fetchAll((query) => listAiTasks(store.id, query, signal)),
      ]);
      const filtered = all.filter((task) => task.status === statusFilter);
      return {
        tools,
        tasks: filtered.slice(offset, offset + PAGE_SIZE),
        total: filtered.length,
      };
    },
    [canRead, mode, store, demoTools, demoTasks, statusFilter, page],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoTools,
    demoTasks,
    statusFilter,
    page,
    canRead,
  ]);

  const act = async (task: AiTask, action: "approve" | "reject") => {
    if (actionBusy) return;
    setActionBusy(true);
    try {
      if (mode === "demo") {
        const updated: AiTask = {
          ...task,
          status: action === "approve" ? "running" : "cancelled",
          updated_at: Math.floor(Date.now() / 1000),
        };
        setDemoTasks((current) => current.map((entry) => (entry.id === task.id ? updated : entry)));
        setDetail((current) => (current?.id === task.id ? updated : current));
        toast.success(
          action === "approve" ? "Task approved in the demo" : "Task rejected in the demo",
        );
      } else {
        if (!store) throw new Error("No store is selected.");
        const updated =
          action === "approve"
            ? await approveAiTask(store.id, task.id)
            : await rejectAiTask(store.id, task.id);
        setDetail((current) => (current?.id === updated.id ? updated : current));
        toast.success(action === "approve" ? "Task approved" : "Task rejected");
      }
      setConfirming(null);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setActionBusy(false);
    }
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const { tools, tasks, total } = data;
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const currentPage = Math.min(page, pages);

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="AI tasks"
        description="Review the tools the agent layer can call, then approve or reject the tasks operators queue against them."
        actions={
          <Button variant="outline" onClick={reload} disabled={loading}>
            <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            Refresh
          </Button>
        }
      />

      <Card className="p-6">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h2 className="text-[14px] font-medium text-foreground">Tools</h2>
            <p className="mt-1 max-w-2xl text-[11.5px] leading-relaxed text-muted-foreground">
              Every tool registered with the agent layer. Mutating tools change store data, and the
              ones that require approval never run until an operator confirms.
            </p>
          </div>
          <p className="shrink-0 text-[11.5px] text-muted-foreground">
            {tools.length} {tools.length === 1 ? "tool" : "tools"}
            {mode === "demo" ? " · sample data" : ""}
          </p>
        </div>
        {tools.length === 0 ? (
          <p className="mt-4 rounded-lg border border-dashed border-border px-3 py-6 text-center text-[12px] text-muted-foreground">
            No tools are registered with the agent layer.
          </p>
        ) : (
          <div className="mt-4 divide-y divide-border">
            {tools.map((tool) => (
              <ToolRow key={tool.name} tool={tool} />
            ))}
          </div>
        )}
      </Card>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div
          role="group"
          aria-label="Filter AI tasks by status"
          className="flex min-w-0 items-center gap-1 overflow-x-auto rounded-full border border-border bg-secondary p-1 no-scrollbar"
        >
          {aiTaskFilterOptions().map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setStatusFilter(option.value);
                setPage(1);
              }}
              aria-pressed={statusFilter === option.value}
              className={cn(
                "flex h-7 shrink-0 cursor-pointer items-center rounded-full px-3 text-[11.5px] font-medium transition-[background-color,color,box-shadow] duration-150",
                statusFilter === option.value
                  ? "bg-card text-foreground shadow-xs"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
        <p className="text-[11.5px] text-muted-foreground">
          {total.toLocaleString("en-US")} {total === 1 ? "task" : "tasks"}
          {mode === "demo" ? " · sample data" : ""}
        </p>
      </div>

      <Card className="overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Task</TableHead>
              <TableHead>Tool</TableHead>
              <TableHead className="w-[130px]">Status</TableHead>
              <TableHead className="hidden w-[170px] md:table-cell">Created by</TableHead>
              <TableHead className="hidden w-[130px] lg:table-cell">Created</TableHead>
              <TableHead className="hidden w-[130px] xl:table-cell">Updated</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Open" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {tasks.map((task) => (
              <TableRow key={task.id} className="h-[58px]">
                <TableCell className="pl-6">
                  <button
                    type="button"
                    onClick={() => setDetail(task)}
                    className="cursor-pointer text-left font-mono text-[11.5px] font-medium text-ink-soft underline-offset-4 hover:text-foreground hover:underline"
                    aria-label={`Open task ${task.id}`}
                  >
                    {task.id}
                  </button>
                </TableCell>
                <TableCell className="font-mono text-[11.5px] text-ink-soft">{task.tool}</TableCell>
                <TableCell>
                  <Badge variant={aiTaskStatusTone(task.status)}>
                    {aiTaskStatusLabel(task.status)}
                  </Badge>
                </TableCell>
                <TableCell className="hidden font-mono text-[11px] text-muted-foreground md:table-cell">
                  {task.created_by ?? "—"}
                </TableCell>
                <TableCell className="hidden text-[11.5px] text-muted-foreground lg:table-cell">
                  {shortDate(fromUnixSeconds(task.created_at))}
                </TableCell>
                <TableCell className="hidden text-[11.5px] text-muted-foreground xl:table-cell">
                  {shortDate(fromUnixSeconds(task.updated_at))}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <ChevronRight className="ml-auto size-4 text-faint" aria-hidden="true" />
                </TableCell>
              </TableRow>
            ))}
            {tasks.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={7} className="p-0">
                  <EmptyState
                    title={statusFilter === "all" ? "No tasks yet" : "No tasks with this status"}
                    description={
                      statusFilter === "all"
                        ? "Tasks appear here when an operator queues a tool invocation through the agent layer."
                        : "Clear the status filter to see every recorded task."
                    }
                    action={
                      statusFilter === "all" ? null : (
                        <Button variant="outline" onClick={() => setStatusFilter("all")}>
                          Show all tasks
                        </Button>
                      )
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>

        <div className="flex flex-col gap-3 border-t border-border px-6 py-3.5 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-[11.5px] text-muted-foreground">
            {total === 0
              ? "No tasks to show"
              : `Showing ${(currentPage - 1) * PAGE_SIZE + 1}–${(currentPage - 1) * PAGE_SIZE + tasks.length} of ${total.toLocaleString("en-US")}`}
          </p>
          <Pagination page={currentPage} pages={pages} onChange={setPage} />
        </div>
      </Card>

      <AiTaskDetailSheet
        task={detail}
        canWrite={canWrite}
        busy={actionBusy}
        onOpenChange={(open) => {
          if (!open) setDetail(null);
        }}
        onApprove={(task) => setConfirming(task)}
        onReject={(task) => void act(task, "reject")}
      />

      <ApproveTaskDialog
        task={confirming}
        busy={actionBusy}
        onCancel={() => {
          if (!actionBusy) setConfirming(null);
        }}
        onConfirm={() => {
          if (confirming) void act(confirming, "approve");
        }}
      />
    </div>
  );
}

function ToolRow({ tool }: { tool: AiTool }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="py-4 first:pt-0 last:pb-0">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <p className="font-mono text-[12.5px] font-medium text-foreground">{tool.name}</p>
          <p className="mt-1 max-w-2xl text-[12px] leading-relaxed text-muted-foreground">
            {tool.description}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-1.5">
          <Badge variant={tool.mutates ? "warning" : "secondary"}>
            {tool.mutates ? "Mutates" : "Read-only"}
          </Badge>
          <Badge variant={tool.requires_approval ? "info" : "outline"}>
            {tool.requires_approval ? "Requires approval" : "No approval"}
          </Badge>
        </div>
      </div>
      <Button
        variant="ghost"
        size="sm"
        className="-ml-2 mt-2"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        aria-label={`${expanded ? "Hide" : "Show"} input schema for ${tool.name}`}
      >
        {expanded ? (
          <ChevronDown className="size-3.5" aria-hidden="true" />
        ) : (
          <ChevronRight className="size-3.5" aria-hidden="true" />
        )}
        Input schema
      </Button>
      {expanded ? <JsonBlock value={tool.input_schema} /> : null}
    </div>
  );
}

function JsonBlock({ value }: { value: unknown }) {
  return (
    <pre className="mt-2 max-h-[360px] overflow-auto rounded-lg border border-border bg-secondary/40 p-3 font-mono text-[11px] leading-relaxed text-ink-soft">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}

function AiTaskDetailSheet({
  task,
  canWrite,
  busy,
  onOpenChange,
  onApprove,
  onReject,
}: {
  task: AiTask | null;
  canWrite: boolean;
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onApprove: (task: AiTask) => void;
  onReject: (task: AiTask) => void;
}) {
  return (
    <Sheet
      open={task !== null}
      onOpenChange={(open) => {
        if (!open) onOpenChange(false);
      }}
    >
      <SheetContent className="flex flex-col sm:max-w-2xl">
        {task ? (
          <>
            <SheetHeader>
              <div className="flex flex-wrap items-center gap-2 pr-8">
                <Badge variant={aiTaskStatusTone(task.status)}>
                  {aiTaskStatusLabel(task.status)}
                </Badge>
                <Badge variant="outline">{task.tool}</Badge>
              </div>
              <SheetTitle className="font-mono text-[14px] break-all">{task.id}</SheetTitle>
              <SheetDescription>
                Requested by {task.created_by ?? "the system"} on{" "}
                {shortDate(fromUnixSeconds(task.created_at))}.
              </SheetDescription>
            </SheetHeader>

            <div className="flex-1 space-y-6 overflow-y-auto p-6">
              <section className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-[11px] font-medium text-muted-foreground">Created by</p>
                  <p className="mt-1 font-mono text-[11.5px] break-all text-ink-soft">
                    {task.created_by ?? "—"}
                  </p>
                </div>
                <div>
                  <p className="text-[11px] font-medium text-muted-foreground">Last updated</p>
                  <p className="mt-1 text-[12.5px] text-ink-soft">
                    {shortDate(fromUnixSeconds(task.updated_at))}
                  </p>
                </div>
              </section>

              <section>
                <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
                  Input
                </p>
                <JsonBlock value={task.input} />
              </section>

              {task.output !== undefined ? (
                <section>
                  <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
                    Output
                  </p>
                  <JsonBlock value={task.output} />
                </section>
              ) : null}

              {task.error ? (
                <section>
                  <p className="text-[11px] font-medium tracking-[0.08em] text-faint uppercase">
                    Error
                  </p>
                  <p className="mt-2 rounded-lg border border-destructive/20 bg-destructive/10 p-3 font-mono text-[11.5px] leading-relaxed break-words text-destructive">
                    {task.error}
                  </p>
                </section>
              ) : null}
            </div>

            <SheetFooter>
              <Button variant="ghost" onClick={() => onOpenChange(false)}>
                Close
              </Button>
              {task.status === "pending" && canWrite ? (
                <>
                  <Button
                    variant="ghost"
                    className="text-destructive hover:text-destructive"
                    onClick={() => onReject(task)}
                    disabled={busy}
                  >
                    <Ban className="size-4" aria-hidden="true" />
                    Reject
                  </Button>
                  <Button onClick={() => onApprove(task)} disabled={busy}>
                    <Play className="size-4" aria-hidden="true" />
                    Approve
                  </Button>
                </>
              ) : null}
            </SheetFooter>
          </>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

function ApproveTaskDialog({
  task,
  busy,
  onCancel,
  onConfirm,
}: {
  task: AiTask | null;
  busy: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Dialog
      open={task !== null}
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Approve this task?</DialogTitle>
          <DialogDescription>
            Approving runs the <span className="font-mono">{task?.tool}</span> tool with the
            recorded input. The API writes an audit entry for the approval.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={onConfirm} disabled={busy}>
            {busy ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Play className="size-4" aria-hidden="true" />
            )}
            Run task
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function demoAiTools(): AiTool[] {
  return [
    {
      name: "catalog.update_price",
      description: "Set the list price for one product variant.",
      input_schema: {
        type: "object",
        properties: {
          variant_id: { type: "string" },
          amount_minor: { type: "integer", minimum: 0 },
          currency_code: { type: "string" },
        },
        required: ["variant_id", "amount_minor", "currency_code"],
      },
      mutates: true,
      requires_approval: true,
    },
    {
      name: "inventory.adjust",
      description: "Adjust the available quantity of one variant.",
      input_schema: {
        type: "object",
        properties: {
          variant_id: { type: "string" },
          delta: { type: "integer" },
          reason: { type: "string" },
        },
        required: ["variant_id", "delta"],
      },
      mutates: true,
      requires_approval: false,
    },
    {
      name: "orders.lookup",
      description: "Read a single order by its identifier.",
      input_schema: {
        type: "object",
        properties: { order_id: { type: "string" } },
        required: ["order_id"],
      },
      mutates: false,
      requires_approval: false,
    },
  ];
}

function demoAiTasks(): AiTask[] {
  return [
    {
      id: "task_demo_9003",
      tool: "catalog.update_price",
      input: {
        variant_id: "var_ridge_sage",
        amount_minor: 14_900,
        currency_code: "USD",
      },
      status: "pending",
      error: null,
      created_by: "user_demo_owner",
      created_at: daysAgo(1),
      updated_at: daysAgo(1),
    },
    {
      id: "task_demo_9002",
      tool: "orders.lookup",
      input: { order_id: "order_demo_missing" },
      status: "failed",
      error: "No order found with that identifier.",
      created_by: "user_demo_operator",
      created_at: daysAgo(2),
      updated_at: daysAgo(2),
    },
    {
      id: "task_demo_9001",
      tool: "inventory.adjust",
      input: { variant_id: "var_ridge_sage", delta: 5, reason: "Stock count correction" },
      output: { variant_id: "var_ridge_sage", available: 42, adjusted_by: 5 },
      status: "succeeded",
      error: null,
      created_by: "user_demo_owner",
      created_at: daysAgo(3),
      updated_at: daysAgo(3),
    },
  ];
}
