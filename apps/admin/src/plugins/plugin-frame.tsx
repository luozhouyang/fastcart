import { useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";

import { useTheme } from "@/components/theme-provider";
import { useAuth } from "@/lib/auth";
import { API_BASE_URL } from "@/lib/api";
import type { AdminExtension } from "@/plugins/types";

const CHANNEL = () => `fc_${crypto.randomUUID().replaceAll("-", "").slice(0, 24)}`;

interface PluginFrameProps {
  extension: AdminExtension;
  target: string;
  context?: Record<string, unknown>;
  height?: number;
}

function csrfToken(): string | null {
  const match = document.cookie.match(/(?:^|; )fc_csrf=([^;]*)/);
  return match ? decodeURIComponent(match[1] ?? "") : null;
}

export function PluginFrame({ extension, target, context = {}, height }: PluginFrameProps) {
  const { store, user, permissions } = useAuth();
  const { theme } = useTheme();
  const navigate = useNavigate();
  const frameRef = useRef<HTMLIFrameElement>(null);
  const channelRef = useRef(CHANNEL());
  const [frameHeight, setFrameHeight] = useState(height ?? extension.height ?? 320);

  useEffect(() => {
    const frame = frameRef.current;
    if (!frame || !store || !extension.src) return;

    const send = (type: string, payload: unknown) => {
      frame.contentWindow?.postMessage(
        { v: 1, type, channel: channelRef.current, ...(type === "init" ? {} : { payload }) },
        "*",
      );
    };

    const init = () => {
      frame.contentWindow?.postMessage(
        {
          v: 1,
          type: "init",
          channel: channelRef.current,
          payload: {
            target,
            pluginId: extension.plugin_id,
            store: { id: store.id, name: store.name, locale: store.locale },
            theme: { mode: theme },
            context,
            user: {
              id: user?.email ?? "",
              name: user?.name ?? "",
              permissions,
            },
            capabilities: extension.settings ?? {},
          },
        },
        "*",
      );
    };

    const onMessage = (event: MessageEvent) => {
      if (event.source !== frame.contentWindow) return;
      const message = event.data as {
        v?: number;
        type?: string;
        channel?: string;
        id?: string;
        method?: string;
        params?: unknown;
        height?: number;
        to?: string;
        level?: string;
        message?: string;
      } | null;
      if (!message || message.v !== 1 || message.channel !== channelRef.current) return;

      if (message.type === "ready") {
        init();
        return;
      }
      if (message.type === "resize" && typeof message.height === "number") {
        setFrameHeight(Math.min(1_200, Math.max(80, Math.round(message.height))));
        return;
      }
      if (
        message.type === "navigate" &&
        typeof message.to === "string" &&
        message.to.startsWith("/")
      ) {
        void navigate({ to: message.to });
        return;
      }
      if (message.type === "toast" && typeof message.message === "string") {
        if (message.level === "error") toast.error(message.message);
        else if (message.level === "success") toast.success(message.message);
        else toast(message.message);
        return;
      }
      if (message.type === "open:external" && typeof message.to === "string") {
        try {
          const url = new URL(message.to);
          if (url.protocol === "https:")
            window.open(url.toString(), "_blank", "noopener,noreferrer");
        } catch {
          // Ignored: invalid external URL.
        }
        return;
      }
      if (message.type === "rpc" && typeof message.method === "string" && message.id) {
        const requestId = message.id;
        const headers: Record<string, string> = {
          "content-type": "application/json",
          "x-store-id": store.id,
        };
        const csrf = csrfToken();
        if (csrf) headers["x-csrf-token"] = csrf;
        void fetch(`${API_BASE_URL}/api/v1/admin/plugins/${extension.plugin_id}/rpc`, {
          method: "POST",
          credentials: "include",
          headers,
          body: JSON.stringify({ method: message.method, params: message.params ?? {} }),
        })
          .then(async (response) => {
            const body = await response.json().catch(() => null);
            if (!response.ok) {
              throw new Error(body?.error ?? `RPC failed (${response.status})`);
            }
            send("rpc:result", { id: requestId, ok: true, result: body });
          })
          .catch((cause: unknown) => {
            send("rpc:result", {
              id: requestId,
              ok: false,
              error: cause instanceof Error ? cause.message : "RPC failed",
            });
          });
      }
    };

    window.addEventListener("message", onMessage);
    frame.addEventListener("load", init);
    if (frame.contentWindow) init();
    return () => {
      window.removeEventListener("message", onMessage);
      frame.removeEventListener("load", init);
    };
  }, [extension, target, context, store, user, permissions, theme, navigate]);

  if (!extension.src) return null;
  return (
    <iframe
      ref={frameRef}
      src={extension.src}
      title={`${extension.plugin_name} ${extension.title ?? extension.label ?? target}`}
      sandbox="allow-scripts allow-forms"
      referrerPolicy="no-referrer"
      loading="lazy"
      style={{ height: `${frameHeight}px` }}
      className="w-full rounded-lg border border-border bg-background"
    />
  );
}
