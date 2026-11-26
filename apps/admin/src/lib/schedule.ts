const DATE_INPUT_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

/**
 * Converts a `type="date"` value (for example `2026-11-30`) to Unix seconds
 * at midnight UTC. Invalid or missing values return null so a blank input
 * means "no date" instead of an API error.
 */
export function dateInputToUnixSeconds(value: string): number | null {
  const trimmed = value.trim();
  if (!DATE_INPUT_PATTERN.test(trimmed)) return null;
  const millis = Date.parse(`${trimmed}T00:00:00Z`);
  if (Number.isNaN(millis)) return null;
  // Reject dates that roll over, such as 2026-02-31.
  if (new Date(millis).toISOString().slice(0, 10) !== trimmed) return null;
  return Math.floor(millis / 1000);
}

/** Formats Unix seconds for a `type="date"` input. */
export function unixSecondsToDateInput(seconds: number | null): string {
  if (seconds === null) return "";
  return new Date(seconds * 1000).toISOString().slice(0, 10);
}

export type ScheduleState = "open" | "scheduled" | "active" | "ended";

export const SCHEDULE_STATE_LABELS: Record<ScheduleState, string> = {
  open: "No window",
  scheduled: "Scheduled",
  active: "Running",
  ended: "Ended",
};

/** Table label for a classified schedule window. */
export function scheduleStateLabel(state: ScheduleState): string {
  return SCHEDULE_STATE_LABELS[state];
}

/**
 * Classifies a start/end window at `now` (Unix seconds). An unset window is
 * open, a future start is scheduled, a past end is ended, and everything else
 * is active.
 */
export function scheduleState(
  startsAt: number | null,
  endsAt: number | null,
  now: number,
): ScheduleState {
  if (startsAt !== null && now < startsAt) return "scheduled";
  if (endsAt !== null && now >= endsAt) return "ended";
  if (startsAt === null && endsAt === null) return "open";
  return "active";
}
