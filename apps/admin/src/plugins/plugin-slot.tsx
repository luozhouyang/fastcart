import { useAuth } from "@/lib/auth";
import { PluginFrame } from "@/plugins/plugin-frame";
import { usePluginExtensions } from "@/plugins/runtime";

interface PluginSlotProps {
  target: string;
  context?: Record<string, unknown>;
  className?: string;
}

/**
 * Renders up to three enabled extensions for one admin target.
 *
 * Rendering nothing when empty avoids layout shifts.
 */
export function PluginSlot({ target, context = {}, className }: PluginSlotProps) {
  const { can } = useAuth();
  const { extensions } = usePluginExtensions();
  const items = extensions
    .filter((extension) => extension.target === target && extension.src)
    .filter((extension) => can(extension.permission))
    .sort((left, right) => (right.priority ?? 0) - (left.priority ?? 0))
    .slice(0, 3);

  if (items.length === 0) return null;
  return (
    <div className={className ?? "space-y-3"}>
      {items.map((extension) => (
        <section
          key={`${extension.plugin_id}:${extension.target}`}
          className="rounded-xl border border-border bg-card p-3 shadow-sm"
          aria-label={`${extension.plugin_name} extension`}
        >
          <header className="mb-2 flex items-center justify-between gap-3">
            <h3 className="text-[12.5px] font-medium text-foreground">
              {extension.title ?? extension.plugin_name}
            </h3>
            <span className="font-mono text-[10px] text-faint">
              {extension.plugin_id} v{extension.plugin_version}
            </span>
          </header>
          <PluginFrame extension={extension} target={target} context={context} />
        </section>
      ))}
    </div>
  );
}
