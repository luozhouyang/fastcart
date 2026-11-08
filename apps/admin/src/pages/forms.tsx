import { useCallback, useState } from "react";
import { Archive, FileText, MoreHorizontal, Plus } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { InquiryForm } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { InquiryFormStatusPill } from "@/components/status-pill";
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
import { useAuth } from "@/lib/auth";
import { errorMessage } from "@/lib/api";
import { demoInquiryForms } from "@/lib/demo-inquiries";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { archiveInquiryForm, createInquiryForm, loadInquiryForms } from "@/lib/loaders";
import { useAsyncData } from "@/lib/use-async-data";

function newDemoForm(name: string, handle: string, description: string): InquiryForm {
  const now = Math.floor(Date.now() / 1000);
  return {
    id: `iform_demo_${now}`,
    name,
    handle: handle.length > 0 ? handle : `form-${now}`,
    description: description.length > 0 ? description : null,
    status: "draft",
    current_version: 1,
    published_version: null,
    created_at: now,
    updated_at: now,
  };
}

export function FormsPage() {
  const { mode, client, store, dataKey } = useAuth();
  const navigate = useNavigate();
  const [demoForms, setDemoForms] = useState<InquiryForm[]>(() => demoInquiryForms());
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [handle, setHandle] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [archiveTarget, setArchiveTarget] = useState<InquiryForm | null>(null);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") {
        return Promise.resolve({ forms: demoInquiryForms(), total: demoInquiryForms().length });
      }
      if (!client) return Promise.reject(new Error("No store is selected."));
      return loadInquiryForms(client, signal);
    },
    [mode, client],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);
  const forms = mode === "demo" ? demoForms : (data?.forms ?? []);

  const resetCreate = () => {
    setName("");
    setHandle("");
    setDescription("");
  };

  const create = async () => {
    if (busy) return;
    if (name.trim().length === 0) {
      toast.error("Give the form a name.");
      return;
    }
    setBusy(true);
    try {
      if (mode === "demo") {
        const form = newDemoForm(name.trim(), handle.trim(), description.trim());
        setDemoForms((current) => [form, ...current]);
        setCreateOpen(false);
        resetCreate();
        toast.success("Form created in the demo");
        void navigate({ to: "/forms/$formId", params: { formId: form.id } });
      } else {
        if (!client) throw new Error("No store is selected.");
        const form = await createInquiryForm(client, {
          name,
          handle,
          description,
        });
        setCreateOpen(false);
        resetCreate();
        reload();
        toast.success("Form created");
        void navigate({ to: "/forms/$formId", params: { formId: form.id } });
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  const archive = async () => {
    if (!archiveTarget || busy) return;
    setBusy(true);
    try {
      if (mode === "demo") {
        setDemoForms((current) =>
          current.map((form) =>
            form.id === archiveTarget.id
              ? { ...form, status: "archived", updated_at: Math.floor(Date.now() / 1000) }
              : form,
          ),
        );
      } else {
        if (!client) throw new Error("No store is selected.");
        await archiveInquiryForm(client, archiveTarget.id);
        reload();
      }
      toast.success("Form archived");
      setArchiveTarget(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  if (!data && mode !== "demo" && loading) return <TableSkeleton />;
  if (!data && mode !== "demo" && error) return <ErrorState message={error} onRetry={reload} />;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Forms"
        description="Build storefront inquiry forms and publish revisions without touching code."
        actions={
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="size-4" aria-hidden="true" />
            New form
          </Button>
        }
      />

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">All forms</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              {forms.length} {forms.length === 1 ? "form" : "forms"} in this store
            </p>
          </div>
        </div>

        {forms.length === 0 ? (
          <EmptyState
            title="No forms yet"
            description="Create a contact or enquiry form, then embed it on your storefront."
            action={
              <Button onClick={() => setCreateOpen(true)}>
                <Plus className="size-4" aria-hidden="true" />
                Create a form
              </Button>
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-6">Form</TableHead>
                <TableHead className="w-[110px]">Status</TableHead>
                <TableHead className="w-[130px]">Version</TableHead>
                <TableHead className="hidden w-[130px] md:table-cell">Updated</TableHead>
                <TableHead className="w-10 pr-5" aria-label="Actions" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {forms.map((form) => (
                <TableRow
                  key={form.id}
                  className="h-[58px] cursor-pointer hover:bg-accent/40"
                  onClick={() =>
                    void navigate({ to: "/forms/$formId", params: { formId: form.id } })
                  }
                >
                  <TableCell className="pl-6">
                    <div className="flex items-center gap-3">
                      <span className="flex size-8 items-center justify-center rounded-lg bg-brand-soft text-brand-ink">
                        <FileText className="size-4" aria-hidden="true" />
                      </span>
                      <div className="min-w-0">
                        <p className="truncate text-[12.5px] font-medium text-foreground">
                          {form.name}
                        </p>
                        <p className="mt-0.5 truncate font-mono text-[10.5px] text-faint">
                          /{form.handle}
                        </p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <InquiryFormStatusPill status={form.status} />
                  </TableCell>
                  <TableCell className="text-[11.5px] text-muted-foreground">
                    <span className="font-mono">v{form.current_version}</span>
                    {form.published_version ? (
                      <span className="ml-1.5 text-faint">· live v{form.published_version}</span>
                    ) : (
                      <span className="ml-1.5 text-faint">· not published</span>
                    )}
                  </TableCell>
                  <TableCell className="hidden text-[11.5px] text-muted-foreground md:table-cell">
                    {shortDate(fromUnixSeconds(form.updated_at))}
                  </TableCell>
                  <TableCell
                    className="pr-5 text-right"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Actions for ${form.name}`}
                          className="text-faint data-[state=open]:bg-accent"
                        >
                          <MoreHorizontal className="size-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-[190px]">
                        <DropdownMenuLabel>{form.name}</DropdownMenuLabel>
                        <DropdownMenuItem
                          onSelect={() =>
                            void navigate({
                              to: "/forms/$formId",
                              params: { formId: form.id },
                            })
                          }
                        >
                          <FileText aria-hidden="true" />
                          Open editor
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          variant="destructive"
                          disabled={form.status === "archived"}
                          onSelect={() => setArchiveTarget(form)}
                        >
                          <Archive aria-hidden="true" />
                          Archive form
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </Card>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>New form</DialogTitle>
            <DialogDescription>
              Give the form a name and an optional handle. You can build the fields next.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="form-name">Name</Label>
              <Input
                id="form-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="Contact us"
                autoFocus
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="form-handle">Handle</Label>
              <Input
                id="form-handle"
                value={handle}
                onChange={(event) => setHandle(event.target.value)}
                placeholder="contact"
                className="font-mono"
              />
              <p className="text-[10.5px] text-faint">
                Used by the storefront to look up the form. Leave blank to generate one.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="form-description">Description</Label>
              <Input
                id="form-description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                placeholder="Optional internal note"
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateOpen(false)} disabled={busy}>
              Cancel
            </Button>
            <Button onClick={() => void create()} disabled={busy}>
              {busy ? "Creating…" : "Create form"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={archiveTarget !== null}
        onOpenChange={(open) => !open && setArchiveTarget(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Archive {archiveTarget?.name}?</DialogTitle>
            <DialogDescription>
              Published forms stop accepting new submissions. Existing inquiries and revisions are
              kept.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setArchiveTarget(null)} disabled={busy}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void archive()} disabled={busy}>
              Archive form
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
