import { Link, useParams } from "@tanstack/react-router";

import { useAuth } from "@/lib/auth";
import { PluginFrame } from "@/plugins/plugin-frame";
import { usePluginExtensions } from "@/plugins/runtime";

export function PluginPage() {
  const params = useParams({ strict: false }) as { pluginId?: string };
  const pluginId = params.pluginId ?? "";
  const { can } = useAuth();
  const { extensions, loading } = usePluginExtensions();
  const mine = extensions.filter((extension) => extension.plugin_id === pluginId);
  const entry =
    mine.find((extension) => extension.target === "admin.nav.section" && extension.src) ??
    mine.find((extension) => extension.src);

  if (loading) {
    return <p className="text-[13px] text-muted-foreground">Loading plugin…</p>;
  }
  if (!pluginId || mine.length === 0) {
    return (
      <div className="space-y-3">
        <h1 className="font-display text-3xl text-foreground">Plugin not found</h1>
        <p className="text-[13px] text-muted-foreground">
          This plugin is not installed, is disabled, or has no admin UI.
        </p>
        <Link
          to="/plugins"
          className="text-[13px] text-brand-ink underline-offset-4 hover:underline"
        >
          Manage plugins
        </Link>
      </div>
    );
  }

  const name = mine[0]?.plugin_name ?? pluginId;
  return (
    <div className="space-y-4">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <p className="font-mono text-[10.5px] tracking-[0.14em] text-faint">PLUGIN</p>
          <h1 className="mt-1 font-display text-[32px] leading-none text-foreground">{name}</h1>
          <p className="mt-2 text-[12px] text-muted-foreground">
            {pluginId} v{mine[0]?.plugin_version}
          </p>
        </div>
        <Link
          to="/plugins"
          className="text-[12.5px] text-brand-ink underline-offset-4 hover:underline"
        >
          Plugin settings
        </Link>
      </header>

      {entry?.src && can(entry.permission) ? (
        <PluginFrame
          extension={entry}
          target={entry.target}
          context={{}}
          height={Math.max(420, entry.height ?? 520)}
        />
      ) : (
        <div className="rounded-xl border border-border bg-card p-6">
          <p className="text-[13px] text-muted-foreground">
            This plugin has admin panels but no full-page entry. Open its configured slots from the
            pages listed below.
          </p>
          <ul className="mt-3 space-y-1 text-[12.5px]">
            {mine
              .filter((extension) => extension.src)
              .map((extension) => (
                <li key={extension.target}>
                  <a
                    href={extension.src ?? "#"}
                    target="_blank"
                    rel="noreferrer"
                    className="text-brand-ink underline-offset-4 hover:underline"
                  >
                    {extension.title ?? extension.target}
                  </a>
                </li>
              ))}
          </ul>
        </div>
      )}
    </div>
  );
}
