import { Boxes, Plus } from "lucide-react";
import type { ThemeEditorManifest } from "@fastcart/shared-types";

import { paletteGroups } from "@/lib/theme-editor";

export function ComponentPalette({
  manifest,
  onAdd,
}: {
  manifest: ThemeEditorManifest | null;
  onAdd: (type: string) => void;
}) {
  const groups = paletteGroups(manifest);

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2 px-1">
        <Boxes className="size-3.5 text-faint" aria-hidden="true" />
        <p className="text-[11px] font-medium text-muted-foreground">Component palette</p>
      </div>
      {groups.length === 0 ? (
        <p className="rounded-lg border border-dashed border-border px-3 py-5 text-center text-[11.5px] text-muted-foreground">
          This theme does not expose editor components.
        </p>
      ) : (
        groups.map((group) => (
          <div key={group.key} className="space-y-1.5">
            <p className="px-1 text-[10.5px] font-medium tracking-[0.06em] text-faint uppercase">
              {group.title}
            </p>
            <div className="grid grid-cols-2 gap-1.5 xl:grid-cols-1">
              {group.entries.map((entry) => (
                <button
                  key={entry.type}
                  type="button"
                  onClick={() => onAdd(entry.type)}
                  title={`Add ${entry.title}`}
                  className="flex h-8 cursor-pointer items-center gap-2 rounded-lg border border-transparent px-2.5 text-left text-[12px] text-ink-soft transition-colors hover:border-border hover:bg-card hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/30 focus-visible:outline-none"
                >
                  <Plus className="size-3.5 shrink-0 text-faint" aria-hidden="true" />
                  <span className="truncate">{entry.title}</span>
                </button>
              ))}
            </div>
          </div>
        ))
      )}
    </div>
  );
}
