import { describe, expect, it } from "vitest";

import {
  SCHEDULE_STATE_LABELS,
  dateInputToUnixSeconds,
  scheduleState,
  scheduleStateLabel,
  unixSecondsToDateInput,
} from "@/lib/schedule";

describe("date inputs", () => {
  it("converts a date input to midnight UTC", () => {
    expect(dateInputToUnixSeconds("2026-11-30")).toBe(Date.UTC(2026, 10, 30) / 1000);
    expect(unixSecondsToDateInput(Date.UTC(2026, 10, 30) / 1000)).toBe("2026-11-30");
  });

  it("returns null for empty, malformed or rolled-over dates", () => {
    expect(dateInputToUnixSeconds("")).toBeNull();
    expect(dateInputToUnixSeconds("30/11/2026")).toBeNull();
    expect(dateInputToUnixSeconds("2026-02-31")).toBeNull();
    expect(unixSecondsToDateInput(null)).toBe("");
  });
});

describe("schedule windows", () => {
  const now = 1_000_000;

  it("classifies unset, future and past windows", () => {
    expect(scheduleState(null, null, now)).toBe("open");
    expect(scheduleState(now + 10, null, now)).toBe("scheduled");
    expect(scheduleState(null, now, now)).toBe("ended");
    expect(scheduleState(now - 10, now + 10, now)).toBe("active");
  });

  it("labels every state", () => {
    expect(scheduleStateLabel("open")).toBe(SCHEDULE_STATE_LABELS.open);
    expect(scheduleStateLabel("scheduled")).toBe("Scheduled");
    expect(scheduleStateLabel("active")).toBe("Running");
    expect(scheduleStateLabel("ended")).toBe("Ended");
  });
});
