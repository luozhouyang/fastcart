import { useEffect, useState } from "react";
import type { NotificationChannel, NotificationChannelEvent } from "@fastcart/shared-types";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

export interface ChannelDraft {
  name: string;
  endpointUrl: string;
  signingSecret: string;
  events: string[];
  isActive: boolean;
}

export function emptyChannelDraft(): ChannelDraft {
  return {
    name: "",
    endpointUrl: "",
    signingSecret: "",
    events: [],
    isActive: true,
  };
}

export function channelToDraft(channel: NotificationChannel): ChannelDraft {
  return {
    name: channel.name,
    endpointUrl: "",
    signingSecret: "",
    events: [...channel.events],
    isActive: channel.is_active,
  };
}

export function NotificationChannelDialog({
  open,
  channel,
  events,
  busy,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  channel: NotificationChannel | null;
  events: NotificationChannelEvent[];
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (draft: ChannelDraft) => void;
}) {
  const [draft, setDraft] = useState<ChannelDraft>(() => emptyChannelDraft());

  useEffect(() => {
    if (!open) return;
    setDraft(channel ? channelToDraft(channel) : emptyChannelDraft());
  }, [open, channel]);

  const editing = channel !== null;
  const canSubmit =
    draft.name.trim().length > 0 &&
    (editing || draft.endpointUrl.trim().length > 0) &&
    draft.events.length > 0;

  const toggleEvent = (type: string) => {
    setDraft((current) => ({
      ...current,
      events: current.events.includes(type)
        ? current.events.filter((entry) => entry !== type)
        : [...current.events, type],
    }));
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{editing ? "Edit channel" : "New notification channel"}</DialogTitle>
          <DialogDescription>
            Send store events to a Feishu custom bot. The signing secret is write-only.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="channel-name">Name</Label>
            <Input
              id="channel-name"
              value={draft.name}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              placeholder="Store operations"
              autoFocus
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="channel-endpoint">Webhook URL</Label>
            <Input
              id="channel-endpoint"
              value={draft.endpointUrl}
              onChange={(event) => setDraft({ ...draft, endpointUrl: event.target.value })}
              placeholder={
                editing
                  ? `${channel?.endpoint_url_masked ?? "Leave blank to keep the current URL"}`
                  : "https://open.feishu.cn/open-apis/bot/v2/hook/…"
              }
              className="font-mono text-[12px]"
            />
            <p className="text-[10.5px] text-faint">
              {editing
                ? "Leave blank to keep the current endpoint. The stored URL is never shown."
                : "Only Feishu and Lark webhook hosts are accepted."}
            </p>
          </div>

          <div className="space-y-2">
            <Label htmlFor="channel-secret">Signing secret</Label>
            <Input
              id="channel-secret"
              type="password"
              value={draft.signingSecret}
              onChange={(event) => setDraft({ ...draft, signingSecret: event.target.value })}
              placeholder={
                editing && channel?.signing_secret_configured
                  ? "Configured — leave blank to keep"
                  : "Optional"
              }
              autoComplete="new-password"
            />
          </div>

          <div className="space-y-2">
            <Label>Events</Label>
            <div className="grid gap-1.5 sm:grid-cols-2">
              {events.map((event) => {
                const checked = draft.events.includes(event.type);
                return (
                  <label
                    key={event.type}
                    className={cn(
                      "flex cursor-pointer items-start gap-2.5 rounded-lg border px-3 py-2.5 transition-colors",
                      checked
                        ? "border-brand/30 bg-brand-soft/40"
                        : "border-border bg-background hover:border-line-strong",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={checked}
                      onChange={() => toggleEvent(event.type)}
                      className="mt-0.5 size-3.5 accent-[var(--brand)]"
                    />
                    <span className="min-w-0">
                      <span className="block text-[12px] font-medium text-foreground">
                        {event.label}
                      </span>
                      <span className="mt-0.5 block text-[10.5px] leading-relaxed text-faint">
                        {event.description}
                      </span>
                    </span>
                  </label>
                );
              })}
              {events.length === 0 ? (
                <p className="col-span-full rounded-lg border border-dashed border-border px-3 py-3 text-[11.5px] text-muted-foreground">
                  No events are available yet.
                </p>
              ) : null}
            </div>
          </div>

          <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-background px-3.5 py-3">
            <div>
              <p className="text-[12px] font-medium text-foreground">Active</p>
              <p className="mt-0.5 text-[10.5px] text-faint">
                Inactive channels stay configured but do not receive events
              </p>
            </div>
            <Switch
              checked={draft.isActive}
              onCheckedChange={(checked) => setDraft({ ...draft, isActive: checked })}
              aria-label="Channel active"
            />
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => onSubmit(draft)} disabled={busy || !canSubmit}>
            {busy ? "Saving…" : editing ? "Save channel" : "Create channel"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
