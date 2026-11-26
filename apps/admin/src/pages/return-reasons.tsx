import { useCallback, useEffect, useState } from "react";
import { Loader2, MoreHorizontal, Pencil, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import type { CreateReturnReasonRequest, ReturnReason } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
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
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { PERMISSIONS } from "@/lib/permissions";
import {
  createReturnReason,
  deleteReturnReason,
  listReturnReasons,
  returnReasonInputError,
  updateReturnReason,
} from "@/lib/return-reasons";
import { useAsyncData } from "@/lib/use-async-data";

/** Demo rows so the page renders without a server. */
function demoReturnReasons(): ReturnReason[] {
  const now = Math.floor(Date.now() / 1000);
  return [
    {
      id: "rreason_demo_damaged",
      value: "damaged",
      label: "Arrived damaged",
      description: "Packaging or item was damaged in transit.",
      metadata: {},
      created_at: now - 86_400 * 30,
      updated_at: now - 86_400 * 30,
    },
    {
      id: "rreason_demo_wrong_size",
      value: "wrong_size",
      label: "Wrong size or fit",
      description: null,
      metadata: {},
      created_at: now - 86_400 * 20,
      updated_at: now - 86_400 * 20,
    },
    {
      id: "rreason_demo_changed_mind",
      value: "changed_mind",
      label: "Customer changed their mind",
      description: null,
      metadata: {},
      created_at: now - 86_400 * 10,
      updated_at: now - 86_400 * 10,
    },
  ];
}

interface ReasonDraft {
  value: string;
  label: string;
  description: string;
}

export function ReturnReasonsPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoReasons, setDemoReasons] = useState<ReturnReason[]>(demoReturnReasons);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<ReturnReason | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<ReturnReason | null>(null);
  const [busy, setBusy] = useState(false);

  const writable = can(PERMISSIONS.orders.write);

  useEffect(() => {
    setDialogOpen(false);
    setEditing(null);
    setDeleteTarget(null);
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<ReturnReason[]> => {
      if (mode === "demo") return Promise.resolve(demoReasons);
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listReturnReasons(store.id, signal);
    },
    [mode, store, demoReasons],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoReasons,
  ]);

  const save = async (draft: ReasonDraft) => {
    setBusy(true);
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        if (editing) {
          setDemoReasons((current) =>
            current.map((reason) =>
              reason.id === editing.id
                ? {
                    ...reason,
                    value: draft.value,
                    label: draft.label,
                    description: draft.description.length === 0 ? null : draft.description,
                    updated_at: now,
                  }
                : reason,
            ),
          );
          toast.success("Return reason updated in the demo");
        } else {
          setDemoReasons((current) => [
            ...current,
            {
              id: `rreason_demo_${Date.now()}`,
              value: draft.value,
              label: draft.label,
              description: draft.description.length === 0 ? null : draft.description,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
          ]);
          toast.success("Return reason created in the demo");
        }
        setDialogOpen(false);
        return;
      }
      if (!store) throw new Error("No store is selected.");
      if (editing) {
        await updateReturnReason(store.id, editing.id, {
          value: draft.value,
          label: draft.label,
          description: draft.description.length === 0 ? null : draft.description,
        });
        toast.success("Return reason updated");
      } else {
        const input: CreateReturnReasonRequest = {
          value: draft.value,
          label: draft.label,
          ...(draft.description.length > 0 ? { description: draft.description } : {}),
        };
        await createReturnReason(store.id, input);
        toast.success("Return reason created");
      }
      setDialogOpen(false);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const confirmDelete = async () => {
    if (!deleteTarget || busy) return;
    const target = deleteTarget;
    setBusy(true);
    try {
      if (mode === "demo") {
        setDemoReasons((current) => current.filter((reason) => reason.id !== target.id));
      } else {
        if (!store) throw new Error("No store is selected.");
        await deleteReturnReason(store.id, target.id);
      }
      setDeleteTarget(null);
      toast.success("Return reason deleted");
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Return reasons"
        description="Configure the reasons operators pick when they create a return, claim or exchange. Existing records keep the label they were given."
        actions={
          writable ? (
            <Button
              onClick={() => {
                setEditing(null);
                setDialogOpen(true);
              }}
            >
              <Plus className="size-4" aria-hidden="true" />
              New reason
            </Button>
          ) : null
        }
      />

      <Card className="overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Label</TableHead>
              <TableHead className="hidden w-[200px] md:table-cell">Value</TableHead>
              <TableHead className="hidden lg:table-cell">Description</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.map((reason) => (
              <TableRow key={reason.id} className="h-[56px]">
                <TableCell className="pl-6 text-[12.5px] font-medium text-foreground">
                  {reason.label}
                </TableCell>
                <TableCell className="hidden font-mono text-[11px] text-muted-foreground md:table-cell">
                  {reason.value}
                </TableCell>
                <TableCell className="hidden max-w-[420px] truncate text-[11.5px] text-muted-foreground lg:table-cell">
                  {reason.description ?? "—"}
                </TableCell>
                <TableCell className="pr-5 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={`Actions for ${reason.label}`}
                        className="text-faint data-[state=open]:bg-accent"
                      >
                        <MoreHorizontal className="size-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-[220px]">
                      <DropdownMenuLabel className="truncate">{reason.label}</DropdownMenuLabel>
                      <DropdownMenuItem
                        disabled={!writable}
                        onSelect={() => {
                          setEditing(reason);
                          setDialogOpen(true);
                        }}
                      >
                        <Pencil aria-hidden="true" />
                        Edit reason
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        variant="destructive"
                        disabled={!writable}
                        onSelect={() => setDeleteTarget(reason)}
                      >
                        <Trash2 aria-hidden="true" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
            {data.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={4} className="p-0">
                  <EmptyState
                    title="No return reasons yet"
                    description="Add reasons so operators pick a consistent label instead of typing one each time."
                    action={
                      writable ? (
                        <Button
                          onClick={() => {
                            setEditing(null);
                            setDialogOpen(true);
                          }}
                        >
                          <Plus className="size-4" aria-hidden="true" />
                          New reason
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

      <ReasonDialog
        open={dialogOpen}
        onOpenChange={(open) => {
          setDialogOpen(open);
          if (!open) setEditing(null);
        }}
        reason={editing}
        busy={busy}
        onSubmit={save}
      />

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setDeleteTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete return reason?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.label}” is removed from the picker. Existing returns, claims and
              exchanges keep the label they were created with.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteTarget(null)} disabled={busy}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void confirmDelete()} disabled={busy}>
              {busy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function ReasonDialog({
  open,
  onOpenChange,
  reason,
  busy,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  reason: ReturnReason | null;
  busy: boolean;
  onSubmit: (draft: ReasonDraft) => Promise<void>;
}) {
  const [value, setValue] = useState("");
  const [label, setLabel] = useState("");
  const [description, setDescription] = useState("");
  const [fieldError, setFieldError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setValue(reason?.value ?? "");
    setLabel(reason?.label ?? "");
    setDescription(reason?.description ?? "");
    setFieldError(null);
  }, [open, reason?.id]);

  const submit = async () => {
    if (busy) return;
    const message = returnReasonInputError({ value, label });
    if (message !== null) {
      setFieldError(message);
      return;
    }
    await onSubmit({ value: value.trim(), label: label.trim(), description: description.trim() });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{reason ? "Edit return reason" : "New return reason"}</DialogTitle>
            <DialogDescription>
              The value is unique per store; the label is snapshotted onto new after-sales records.
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="return-reason-value">Value</Label>
              <Input
                id="return-reason-value"
                value={value}
                onChange={(event) => {
                  setValue(event.target.value);
                  setFieldError(null);
                }}
                placeholder="damaged"
                autoComplete="off"
                spellCheck={false}
                disabled={busy}
                className="font-mono"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="return-reason-label">Label</Label>
              <Input
                id="return-reason-label"
                value={label}
                onChange={(event) => {
                  setLabel(event.target.value);
                  setFieldError(null);
                }}
                placeholder="Arrived damaged"
                autoComplete="off"
                autoFocus
                disabled={busy}
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="return-reason-description">Description (optional)</Label>
            <textarea
              id="return-reason-description"
              rows={3}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              disabled={busy}
              placeholder="Packaging or item was damaged in transit."
              className="w-full resize-none rounded-md border border-input bg-card px-3 py-2 text-[13.5px] text-foreground shadow-xs transition-[color,box-shadow,border-color] outline-none placeholder:text-faint hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25 disabled:opacity-50"
            />
          </div>

          {fieldError ? (
            <p className="text-[11.5px] text-destructive" role="alert">
              {fieldError}
            </p>
          ) : null}

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
              {reason ? "Save changes" : "Create reason"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
