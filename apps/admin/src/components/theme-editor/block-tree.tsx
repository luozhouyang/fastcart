import { useState } from "react";
import { ChevronDown, ChevronUp, Layers, Plus, Trash2 } from "lucide-react";
import type { PageBlock, ThemeEditorManifest, ThemeEditorSlot } from "@fastcart/shared-types";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { blockId, componentSlots, componentTitle, slotEntries } from "@/lib/theme-editor";

function SlotAdd({
  manifest,
  slot,
  onAdd,
}: {
  manifest: ThemeEditorManifest | null;
  slot: ThemeEditorSlot;
  onAdd: (type: string) => void;
}) {
  const entries = slotEntries(manifest, slot);
  const [selected, setSelected] = useState(entries[0]?.type ?? "");

  if (entries.length === 0) {
    return (
      <p className="px-2 py-1 text-[10.5px] text-faint">No components are allowed in this slot.</p>
    );
  }

  return (
    <div className="flex items-center gap-1.5 px-2 py-1.5">
      <select
        value={selected}
        onChange={(event) => setSelected(event.target.value)}
        aria-label={`Component to add to ${slot.title ?? slot.name}`}
        className="h-7 min-w-0 flex-1 cursor-pointer rounded-md border border-input bg-card px-2 text-[11.5px] text-foreground outline-none hover:border-line-strong focus-visible:border-ring"
      >
        {entries.map((entry) => (
          <option key={entry.type} value={entry.type}>
            {entry.title}
          </option>
        ))}
      </select>
      <Button
        type="button"
        variant="outline"
        size="sm"
        className="h-7 px-2 text-[11px]"
        disabled={selected.length === 0}
        onClick={() => selected.length > 0 && onAdd(selected)}
      >
        <Plus className="size-3" aria-hidden="true" />
        Add
      </Button>
    </div>
  );
}

function BlockList({
  blocks,
  manifest,
  selectedId,
  onSelect,
  onMove,
  onRemove,
  onAddToSlot,
  depth,
}: {
  blocks: PageBlock[];
  manifest: ThemeEditorManifest | null;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onMove: (id: string, direction: -1 | 1) => void;
  onRemove: (id: string) => void;
  onAddToSlot: (parentId: string, slotName: string, type: string) => void;
  depth: number;
}) {
  if (blocks.length === 0) {
    return (
      <p className="rounded-lg border border-dashed border-border px-3 py-3 text-center text-[11px] text-faint">
        No blocks yet.
      </p>
    );
  }

  return (
    <div className={cn("space-y-1.5", depth > 0 ? "border-l border-border/70 pl-3" : "")}>
      {blocks.map((block, index) => {
        const id = blockId(block);
        const active = id !== null && id === selectedId;
        const slots = componentSlots(manifest, block.type);
        return (
          <div key={id ?? `${block.type}-${index}`} className="space-y-1">
            <div
              role="button"
              tabIndex={0}
              onClick={() => id && onSelect(id)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && id) onSelect(id);
              }}
              className={cn(
                "flex cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-2 transition-colors outline-none",
                active
                  ? "border-brand/40 bg-brand-soft/50"
                  : "border-border bg-card hover:border-line-strong",
              )}
            >
              <Layers
                className={cn("size-3.5 shrink-0", active ? "text-brand-ink" : "text-faint")}
                aria-hidden="true"
              />
              <div className="min-w-0 flex-1">
                <p className="truncate text-[12px] font-medium text-foreground">
                  {componentTitle(manifest, block.type)}
                </p>
                <p className="truncate font-mono text-[10px] text-faint">
                  {id ?? "missing id"} · {block.type}
                </p>
              </div>
              <div
                className="flex items-center gap-0.5"
                onClick={(event) => event.stopPropagation()}
              >
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="size-6 text-faint"
                  disabled={index === 0}
                  aria-label="Move block up"
                  onClick={() => id && onMove(id, -1)}
                >
                  <ChevronUp className="size-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="size-6 text-faint"
                  disabled={index === blocks.length - 1}
                  aria-label="Move block down"
                  onClick={() => id && onMove(id, 1)}
                >
                  <ChevronDown className="size-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="size-6 text-faint hover:text-destructive"
                  aria-label="Delete block"
                  onClick={() => id && onRemove(id)}
                >
                  <Trash2 className="size-3.5" />
                </Button>
              </div>
            </div>

            {slots.length > 0 ? (
              <div className="ml-4 space-y-2">
                {slots.map((slot) => (
                  <div key={slot.name} className="space-y-1">
                    <p className="px-2 text-[10px] font-medium tracking-[0.05em] text-faint uppercase">
                      {slot.title ?? slot.name}
                    </p>
                    <BlockList
                      blocks={block.slots?.[slot.name] ?? []}
                      manifest={manifest}
                      selectedId={selectedId}
                      onSelect={onSelect}
                      onMove={onMove}
                      onRemove={onRemove}
                      onAddToSlot={onAddToSlot}
                      depth={depth + 1}
                    />
                    {id ? (
                      <SlotAdd
                        manifest={manifest}
                        slot={slot}
                        onAdd={(type) => onAddToSlot(id, slot.name, type)}
                      />
                    ) : null}
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

export function BlockTree({
  blocks,
  manifest,
  selectedId,
  onSelect,
  onMove,
  onRemove,
  onAddToSlot,
}: {
  blocks: PageBlock[];
  manifest: ThemeEditorManifest | null;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onMove: (id: string, direction: -1 | 1) => void;
  onRemove: (id: string) => void;
  onAddToSlot: (parentId: string, slotName: string, type: string) => void;
}) {
  return (
    <BlockList
      blocks={blocks}
      manifest={manifest}
      selectedId={selectedId}
      onSelect={onSelect}
      onMove={onMove}
      onRemove={onRemove}
      onAddToSlot={onAddToSlot}
      depth={0}
    />
  );
}
