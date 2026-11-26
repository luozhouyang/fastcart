import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  Check,
  Copy,
  FileText,
  Image as ImageIcon,
  Loader2,
  MoreHorizontal,
  RefreshCw,
  Search,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { toast } from "sonner";
import type { Asset } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, PageSkeleton } from "@/components/states";
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
import { Input } from "@/components/ui/input";
import { API_BASE_URL, errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { fromUnixSeconds, shortDate } from "@/lib/format";
import {
  assetUrl,
  deleteAsset,
  dimensionLabel,
  formatBytes,
  isAcceptedFile,
  isPreviewable,
  listAssets,
  uploadAsset,
} from "@/lib/media";
import { useAsyncData } from "@/lib/use-async-data";
import { cn } from "@/lib/utils";

const DEMO_ASSETS: Asset[] = [
  {
    id: "asset_demo_carafe",
    filename: "ridge-carafe-hero.png",
    content_type: "image/png",
    size_bytes: 2_412_000,
    storage_key: "assets/demo-store/asset_demo_carafe/ridge-carafe-hero.png",
    alt: "Ridge ceramic carafe on a linen cloth",
    width: 1600,
    height: 1200,
    created_by: "user_demo",
    created_at: 1_760_000_000,
    deleted_at: null,
  },
  {
    id: "asset_demo_lamp",
    filename: "arc-table-lamp.jpg",
    content_type: "image/jpeg",
    size_bytes: 894_000,
    storage_key: "assets/demo-store/asset_demo_lamp/arc-table-lamp.jpg",
    alt: "Arc table lamp with a warm glow",
    width: 1200,
    height: 1500,
    created_by: "user_demo",
    created_at: 1_760_100_000,
    deleted_at: null,
  },
  {
    id: "asset_demo_lookbook",
    filename: "autumn-lookbook.pdf",
    content_type: "application/pdf",
    size_bytes: 5_020_000,
    storage_key: "assets/demo-store/asset_demo_lookbook/autumn-lookbook.pdf",
    alt: null,
    width: null,
    height: null,
    created_by: "user_demo",
    created_at: 1_760_200_000,
    deleted_at: null,
  },
];

function demoAssetRows(): Asset[] {
  return DEMO_ASSETS.map((asset) => ({ ...asset }));
}

/**
 * Absolute asset URL.
 *
 * In development the admin runs on the Vite port while the API serves bytes
 * from another origin, so the documented path is resolved against the API base
 * URL when one is configured.
 */
function assetHref(asset: Asset): string {
  const path = assetUrl(asset.id);
  return API_BASE_URL.length > 0 ? `${API_BASE_URL}${path}` : path;
}

/** CSS background used for image previews; demo assets have no real bytes. */
function previewStyle(asset: Asset): CSSProperties {
  if (!isPreviewable(asset)) return {};
  if (asset.id.startsWith("asset_demo_")) {
    const tone = asset.id.endsWith("lamp") ? "#D9CBB2" : "#A9BFAE";
    return { backgroundImage: `linear-gradient(135deg, ${tone}, #F6F2EA)` };
  }
  return { backgroundImage: `url("${assetHref(asset)}")` };
}

export function MediaPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoAssets, setDemoAssets] = useState<Asset[]>(demoAssetRows);
  const [search, setSearch] = useState("");
  const [dragging, setDragging] = useState(false);
  const [uploads, setUploads] = useState<Record<string, number>>({});
  const [deleteTarget, setDeleteTarget] = useState<Asset | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setSearch("");
    setDeleteTarget(null);
    setUploads({});
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<Asset[]> => {
      if (mode === "demo") return Promise.resolve(demoAssets);
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listAssets(store.id, { limit: 100 }, signal).then((page) => page.data);
    },
    [mode, store, demoAssets],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoAssets,
  ]);

  const assets = useMemo(() => data ?? [], [data]);
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (needle.length === 0) return assets;
    return assets.filter(
      (asset) =>
        asset.filename.toLowerCase().includes(needle) ||
        (asset.alt ?? "").toLowerCase().includes(needle),
    );
  }, [assets, search]);

  const uploadFiles = async (files: FileList | File[]) => {
    const list = Array.from(files);
    if (list.length === 0) return;
    if (!can("themes.write")) {
      toast.error("You do not have permission to upload media.");
      return;
    }
    for (const file of list) {
      if (!isAcceptedFile(file)) {
        toast.error(`${file.name} is not an image, video or PDF.`);
        continue;
      }
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        setDemoAssets((current) => [
          {
            id: `asset_demo_${Date.now()}_${current.length}`,
            filename: file.name,
            content_type: file.type.length > 0 ? file.type : "image/png",
            size_bytes: file.size,
            storage_key: `assets/demo-store/${file.name}`,
            alt: null,
            width: null,
            height: null,
            created_by: "user_demo",
            created_at: now,
            deleted_at: null,
          },
          ...current,
        ]);
        toast.success(`${file.name} added to the demo library`);
        continue;
      }
      if (!store) return;
      const key = `${file.name}-${Date.now()}`;
      setUploads((current) => ({ ...current, [key]: 0 }));
      try {
        const asset = await uploadAsset(store.id, file, { alt: "" });
        toast.success(`${asset.filename} uploaded`);
        reload();
      } catch (cause) {
        toast.error(errorMessage(cause));
      } finally {
        setUploads((current) => {
          const next = { ...current };
          delete next[key];
          return next;
        });
      }
    }
  };

  const copyUrl = async (asset: Asset) => {
    // The stored URL is only meaningful when the admin is served same-origin
    // with the API; otherwise the configured API origin prefixes the path.
    const copyValue =
      API_BASE_URL.length > 0 ? assetHref(asset) : `${window.location.origin}${assetUrl(asset.id)}`;
    try {
      await navigator.clipboard.writeText(copyValue);
      setCopiedId(asset.id);
      toast.success("Asset URL copied");
      window.setTimeout(() => {
        setCopiedId((current) => (current === asset.id ? null : current));
      }, 1500);
    } catch {
      toast.error("Could not copy automatically. Select the URL and copy it manually.");
    }
  };

  const confirmDelete = async () => {
    if (!deleteTarget || deleteBusy) return;
    const target = deleteTarget;
    setDeleteBusy(true);
    try {
      if (mode === "demo") {
        setDemoAssets((current) => current.filter((asset) => asset.id !== target.id));
      } else {
        if (!store) throw new Error("No store is selected.");
        await deleteAsset(store.id, target.id);
      }
      setDeleteTarget(null);
      toast.success(`${target.filename} deleted`);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setDeleteBusy(false);
    }
  };

  if (!data && loading) return <PageSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const uploading = Object.keys(uploads).length > 0;
  const writable = can("themes.write");

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Media"
        description="Upload images and files once, then reference them from products and pages by URL."
        actions={
          <>
            <Button
              variant="outline"
              size="icon"
              onClick={reload}
              disabled={loading}
              aria-label="Refresh media library"
              title="Refresh media library"
            >
              <RefreshCw className={cn("size-4", loading && "animate-spin")} aria-hidden="true" />
            </Button>
            <Button
              onClick={() => fileInputRef.current?.click()}
              disabled={!writable || uploading}
              title={writable ? undefined : "Requires themes.write"}
            >
              {uploading ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Upload className="size-4" aria-hidden="true" />
              )}
              Upload
            </Button>
          </>
        }
      />

      <input
        ref={fileInputRef}
        type="file"
        accept="image/*,video/mp4,application/pdf"
        multiple
        className="sr-only"
        aria-label="Upload media files"
        onChange={(event) => {
          const files = event.target.files;
          if (files) void uploadFiles(files);
          event.target.value = "";
        }}
      />

      <div
        onDragOver={(event) => {
          event.preventDefault();
          if (writable) setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          if (!writable) {
            toast.error("You do not have permission to upload media.");
            return;
          }
          if (event.dataTransfer.files.length > 0) void uploadFiles(event.dataTransfer.files);
        }}
        className={cn(
          "rounded-xl border border-dashed p-6 text-center transition-colors",
          dragging
            ? "border-brand/50 bg-brand-soft/40"
            : "border-border bg-secondary/20 hover:border-line-strong",
        )}
      >
        <span className="mx-auto flex size-10 items-center justify-center rounded-full bg-card text-muted-foreground shadow-xs">
          <Upload className="size-4" aria-hidden="true" />
        </span>
        <p className="mt-3 text-[12.5px] font-medium text-foreground">Drop files here to upload</p>
        <p className="mt-1 text-[11.5px] text-muted-foreground">
          PNG, JPEG, WebP, GIF, MP4 and PDF. The API detects the type from the bytes.
        </p>
        <Button
          variant="outline"
          size="sm"
          className="mt-3"
          onClick={() => fileInputRef.current?.click()}
          disabled={!writable || uploading}
        >
          <Upload className="size-3.5" aria-hidden="true" />
          Choose files
        </Button>
      </div>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative w-full sm:w-[320px]">
          <Search
            className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
            aria-hidden="true"
          />
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search by filename or alt text"
            className="pr-9 pl-9"
            aria-label="Search media"
          />
          {search.length > 0 ? (
            <button
              type="button"
              onClick={() => setSearch("")}
              className="absolute top-1/2 right-2.5 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-faint hover:bg-accent hover:text-foreground"
              aria-label="Clear search"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>
        <p className="text-[11.5px] text-muted-foreground">
          {assets.length} {assets.length === 1 ? "asset" : "assets"}
          {mode === "demo" ? " · sample data" : ""}
        </p>
      </div>

      {filtered.length === 0 ? (
        <Card className="p-0">
          <EmptyState
            title={assets.length === 0 ? "No media yet" : "No matching media"}
            description={
              assets.length === 0
                ? "Upload an image to reference it from products, pages and themes."
                : "Try a different filename or alt text."
            }
            action={
              assets.length === 0 && writable ? (
                <Button onClick={() => fileInputRef.current?.click()}>
                  <Upload className="size-4" aria-hidden="true" />
                  Upload media
                </Button>
              ) : null
            }
          />
        </Card>
      ) : (
        <section
          aria-label="Media library"
          className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4"
        >
          {filtered.map((asset) => {
            const dimensions = dimensionLabel(asset);
            return (
              <Card key={asset.id} className="overflow-hidden p-0">
                <div
                  className="relative flex aspect-[4/3] items-center justify-center bg-secondary/50 bg-cover bg-center"
                  style={previewStyle(asset)}
                  role="img"
                  aria-label={asset.alt ?? asset.filename}
                >
                  {!isPreviewable(asset) ? (
                    <span className="flex flex-col items-center gap-2 text-muted-foreground">
                      {asset.content_type === "application/pdf" ? (
                        <FileText className="size-8" aria-hidden="true" />
                      ) : (
                        <ImageIcon className="size-8" aria-hidden="true" />
                      )}
                      <span className="font-mono text-[10px] uppercase">
                        {asset.content_type.split("/")[1] ?? asset.content_type}
                      </span>
                    </span>
                  ) : null}
                  <div className="absolute top-2 right-2">
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="secondary"
                          size="icon-sm"
                          aria-label={`Actions for ${asset.filename}`}
                          className="bg-card/90 shadow-xs backdrop-blur"
                        >
                          <MoreHorizontal className="size-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-[200px]">
                        <DropdownMenuLabel className="truncate">{asset.filename}</DropdownMenuLabel>
                        <DropdownMenuItem onSelect={() => void copyUrl(asset)}>
                          {copiedId === asset.id ? (
                            <Check aria-hidden="true" />
                          ) : (
                            <Copy aria-hidden="true" />
                          )}
                          Copy URL
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          variant="destructive"
                          disabled={!writable}
                          onSelect={() => setDeleteTarget(asset)}
                        >
                          <Trash2 aria-hidden="true" />
                          Delete
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </div>
                <div className="p-4">
                  <p className="truncate text-[12.5px] font-medium text-foreground">
                    {asset.filename}
                  </p>
                  <p className="mt-1 truncate text-[10.5px] text-faint">
                    {asset.alt ?? "No alt text"}
                  </p>
                  <div className="mt-3 flex items-center justify-between gap-2 text-[10.5px] text-muted-foreground">
                    <span className="font-mono">{formatBytes(asset.size_bytes)}</span>
                    <span>{dimensions ?? asset.content_type.split("/")[1] ?? "file"}</span>
                  </div>
                  <div className="mt-3 flex items-center justify-between gap-2">
                    <span className="text-[10.5px] text-faint">
                      {shortDate(fromUnixSeconds(asset.created_at))}
                    </span>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => void copyUrl(asset)}
                      aria-label={`Copy URL for ${asset.filename}`}
                    >
                      {copiedId === asset.id ? (
                        <Check className="size-3.5" aria-hidden="true" />
                      ) : (
                        <Copy className="size-3.5" aria-hidden="true" />
                      )}
                      {copiedId === asset.id ? "Copied" : "Copy URL"}
                    </Button>
                  </div>
                </div>
              </Card>
            );
          })}
        </section>
      )}

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deleteBusy) setDeleteTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete asset?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.filename}” is removed from the library and its stored file is purged.
              Pages that reference its URL will show a broken image.
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
              Delete asset
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
