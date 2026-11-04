import { describe, expect, it } from "vitest";

import {
  csvEscape,
  dayMonth,
  dayMonthYear,
  fromUnixSeconds,
  initials,
  money,
  moneyShort,
  moneyWhole,
  monthDay,
} from "@/lib/format";

describe("money formatting", () => {
  it("formats minor-unit values with and without cents", () => {
    expect(money(1234.5)).toBe("$1,234.50");
    expect(moneyWhole(1234.5)).toBe("$1,235");
  });

  it("drops cents only for whole amounts", () => {
    expect(moneyShort(58)).toBe("$58");
    expect(moneyShort(58.5)).toBe("$58.50");
  });

  it("honours the store currency", () => {
    expect(money(58, "EUR")).toBe("€58.00");
  });
});

describe("date formatting", () => {
  it("formats day-first labels used by the overview", () => {
    const iso = new Date(2026, 10, 4, 12).toISOString();
    expect(dayMonth(iso)).toBe("4 Nov");
    expect(dayMonthYear(iso)).toBe("4 Nov 2026");
    expect(monthDay(iso)).toBe("Nov 4");
  });

  it("converts Unix seconds to ISO strings", () => {
    expect(fromUnixSeconds(0)).toBe("1970-01-01T00:00:00.000Z");
    expect(fromUnixSeconds(1_793_750_400)).toBe("2026-11-04T00:00:00.000Z");
  });
});

describe("small helpers", () => {
  it("builds initials from the first two words", () => {
    expect(initials("Ava Whitfield")).toBe("AW");
    expect(initials("  ")).toBe("");
  });

  it("escapes CSV fields with commas and quotes", () => {
    expect(csvEscape("Smith, Ava")).toBe('"Smith, Ava"');
    expect(csvEscape('He said "hi"')).toBe('"He said ""hi"""');
    expect(csvEscape(null)).toBe("");
  });
});
