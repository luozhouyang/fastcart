import { useCallback, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Archive, FileText, MoreHorizontal, Plus } from "lucide-react";
import { toast } from "sonner";
import type { Page, PageStatus } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { PageStatusPill } from "@/components/status-pill";
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
import { archiveDemoPage, createDemoPage, listDemoPages } from "@/lib/demo-content";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import { useAsyncData } from "@/lib/use-async-data";

const PAGE_TYPES = [
  { value: "custom", label: "Custom page" },
  { value: "home", label: "Home page" },
  { value: "faq", label: "FAQ" },
] as const;

const selectClass =
  "h-10 w-full cursor-pointer appearance-none rounded-md border border-input bg-card px-3 text-[13.5px] text-foreground shadow-xs outline-none transition-colors hover:border-line-strong focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/25";

export function PagesPage() {
  const { mode, client, store, dataKey } = useAuth();
  const navigate = useNavigate();
  const [demoPages, setDemoPages] = useState<Page[]>(() => listDemoPages());
  const [createOpen, setCreateOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [handle, setHandle] = useState("");
  const [pageType, setPageType] = useState<string>("custom");
  const [busy, setBusy] = useState<"create" | "archive" | null>(null);
  const [archiveTarget, setArchiveTarget] = useState<Page | null>(null);

  const loader = useCallback(
    (signal: AbortSignal) => {
      if (mode === "demo") return Promise.resolve(listDemoPages());
      if (!client) return Promise.reject(new Error("No store is selected."));
      return client.content.pages.list({ limit: 100 }, { signal }).then((page) => page.data);
    },
    [mode, client],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [mode, store?.id, dataKey]);
  const pages = mode === "demo" ? demoPages : (data ?? []);

  const resetCreate = () => {
    setTitle("");
    setHandle("");
    setPageType("custom");
  };

  const create = async () => {
    if (busy) return;
    if (title.trim().length === 0) {
      toast.error("Give the page a title.");
      return;
    }
    setBusy("create");
    try {
      if (mode === "demo") {
        const page = createDemoPage({
          title: title.trim(),
          handle: handle.trim() || null,
          page_type: pageType,
        });
        setDemoPages(listDemoPages());
        setCreateOpen(false);
        resetCreate();
        toast.success("Page created in the demo");
        void navigate({ to: "/pages/$pageId", params: { pageId: page.id } });
      } else {
        if (!client) throw new Error("No store is selected.");
        const page = await client.content.pages.create({
          title: title.trim(),
          handle: handle.trim() || null,
          page_type: pageType,
        });
        setCreateOpen(false);
        resetCreate();
        reload();
        toast.success("Page created");
        void navigate({ to: "/pages/$pageId", params: { pageId: page.id } });
      }
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const archive = async () => {
    if (!archiveTarget || busy) return;
    const target: Page = archiveTarget;
    setBusy("archive");
    try {
      if (mode === "demo") {
        archiveDemoPage(target.id);
        setDemoPages(listDemoPages());
      } else {
        if (!client) throw new Error("No store is selected.");
        await client.content.pages.archive(target.id);
        reload();
      }
      toast.success("Page archived");
      setArchiveTarget(null);
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  if (!data && mode !== "demo" && loading) return <TableSkeleton />;
  if (!data && mode !== "demo" && error) return <ErrorState message={error} onRetry={reload} />;

  const published = pages.filter((page) => page.status === "published").length;

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Pages"
        description="Author storefront pages with the active theme, preview drafts, and publish when ready."
        actions={
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="size-4" aria-hidden="true" />
            New page
          </Button>
        }
      />

      <Card className="overflow-hidden">
        <div className="flex items-start justify-between gap-4 px-6 pt-5 pb-2">
          <div>
            <p className="text-[13.5px] font-medium text-foreground">Content pages</p>
            <p className="mt-1 text-[11.5px] text-muted-foreground">
              {pages.length} {pages.length === 1 ? "page" : "pages"} · {published} published
            </p>
          </div>
        </div>

        {pages.length === 0 ? (
          <EmptyState
            title="No pages yet"
            description="Create a page, then compose it from the components exposed by your active theme."
            action={
              <Button onClick={() => setCreateOpen(true)}>
                <Plus className="size-4" aria-hidden="true" />
                Create a page
              </Button>
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead className="pl-6">Page</TableHead>
                <TableHead className="w-[110px]">Status</TableHead>
                <TableHead className="w-[110px]">Revision</TableHead>
                <TableHead className="hidden w-[130px] md:table-cell">Updated</TableHead>
                <TableHead className="w-10 pr-5" aria-label="Actions" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {pages.map((page) => (
                <TableRow
                  key={page.id}
                  className="h-[58px] cursor-pointer hover:bg-accent/40"
                  onClick={() =>
                    void navigate({ to: "/pages/$pageId", params: { pageId: page.id } })
                  }
                >
                  <TableCell className="pl-6">
                    <div className="flex items-center gap-3">
                      <span className="flex size-8 items-center justify-center rounded-lg bg-brand-soft text-brand-ink">
                        <FileText className="size-4" aria-hidden="true" />
                      </span>
                      <div className="min-w-0">
                        <p className="truncate text-[12.5px] font-medium text-foreground">
                          {page.title}
                        </p>
                        <p className="mt-0.5 truncate font-mono text-[10.5px] text-faint">
                          /{page.handle} · {page.page_type}
                        </p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell>
                    <PageStatusPill status={page.status as PageStatus} />
                  </TableCell>
                  <TableCell className="text-[11.5px] text-muted-foreground">
                    <span className="font-mono">r{page.revision}</span>
                  </TableCell>
                  <TableCell className="hidden text-[11.5px] text-muted-foreground md:table-cell">
                    {shortDate(fromUnixSeconds(page.updated_at))}
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
                          aria-label={`Actions for ${page.title}`}
                          className="text-faint data-[state=open]:bg-accent"
                        >
                          <MoreHorizontal className="size-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-[190px]">
                        <DropdownMenuLabel>{page.title}</DropdownMenuLabel>
                        <DropdownMenuItem
                          onSelect={() =>
                            void navigate({
                              to: "/pages/$pageId",
                              params: { pageId: page.id },
                            })
                          }
                        >
                          <FileText aria-hidden="true" />
                          Open editor
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          variant="destructive"
                          disabled={page.status === "archived"}
                          onSelect={() => setArchiveTarget(page)}
                        >
                          <Archive aria-hidden="true" />
                          Archive page
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
            <DialogTitle>New page</DialogTitle>
            <DialogDescription>
              Start with a title and handle. Components come from the active theme editor.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="page-title">Title</Label>
              <Input
                id="page-title"
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                placeholder="About us"
                autoFocus
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="page-handle">Handle</Label>
              <Input
                id="page-handle"
                value={handle}
                onChange={(event) => setHandle(event.target.value)}
                placeholder="about"
                className="font-mono"
              />
              <p className="text-[10.5px] text-faint">
                Used by the storefront URL. Leave blank to generate one from the title.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="page-type">Page type</Label>
              <select
                id="page-type"
                value={pageType}
                onChange={(event) => setPageType(event.target.value)}
                className={selectClass}
              >
                {PAGE_TYPES.map((entry) => (
                  <option key={entry.value} value={entry.value}>
                    {entry.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateOpen(false)} disabled={busy !== null}>
              Cancel
            </Button>
            <Button onClick={() => void create()} disabled={busy !== null}>
              {busy === "create" ? "Creating…" : "Create page"}
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
            <DialogTitle>Archive {archiveTarget?.title}?</DialogTitle>
            <DialogDescription>
              The page stops rendering on the storefront. Its revisions are kept and it can be
              restored from the API.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setArchiveTarget(null)}
              disabled={busy !== null}
            >
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void archive()} disabled={busy !== null}>
              Archive page
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
