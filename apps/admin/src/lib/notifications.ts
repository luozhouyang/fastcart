import type {
  NotificationChannel,
  NotificationChannelEvent,
  NotificationDeliveryStatus,
} from "@fastcart/shared-types";

export function channelEndpointLabel(
  channel: Pick<NotificationChannel, "endpoint_url_masked"> | null | undefined,
): string {
  const masked = channel?.endpoint_url_masked?.trim();
  return masked !== undefined && masked.length > 0 ? masked : "Endpoint hidden";
}

export function channelSecretLabel(configured: boolean): string {
  return configured ? "Signing secret configured" : "No signing secret";
}

/** Masks the trailing path of a webhook URL for display in the admin. */
export function maskEndpoint(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname.replace(/\/$/, "");
    const visible = path.slice(0, 18);
    return `${parsed.origin}${visible}••••`;
  } catch {
    return "••••";
  }
}

export const DELIVERY_STATUS_LABELS: Record<NotificationDeliveryStatus, string> = {
  pending: "Pending",
  queued: "Queued",
  sent: "Sent",
  failed: "Failed",
  skipped: "Skipped",
};

export const DELIVERY_STATUS_TONES: Record<
  NotificationDeliveryStatus,
  "secondary" | "info" | "brand" | "destructive" | "warning"
> = {
  pending: "secondary",
  queued: "info",
  sent: "brand",
  failed: "destructive",
  skipped: "warning",
};

export function eventLabel(events: NotificationChannelEvent[], type: string): string {
  return events.find((event) => event.type === type)?.label ?? type;
}
