const moneyFormatters = new Map<string, Intl.NumberFormat>();
const wholeMoneyFormatters = new Map<string, Intl.NumberFormat>();

function formatter(
  cache: Map<string, Intl.NumberFormat>,
  currency: string,
  digits: { minimumFractionDigits: number; maximumFractionDigits: number },
): Intl.NumberFormat {
  const key = `${currency}:${digits.minimumFractionDigits}:${digits.maximumFractionDigits}`;
  const existing = cache.get(key);
  if (existing) return existing;
  const created = new Intl.NumberFormat("en-US", {
    style: "currency",
    currency,
    ...digits,
  });
  cache.set(key, created);
  return created;
}

const numberFormatter = new Intl.NumberFormat("en-US");

export function money(value: number, currency = "USD"): string {
  return formatter(moneyFormatters, currency, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

export function moneyWhole(value: number, currency = "USD"): string {
  return formatter(wholeMoneyFormatters, currency, {
    minimumFractionDigits: 0,
    maximumFractionDigits: 0,
  }).format(value);
}

/** Currency with cents only when the value is not a whole amount. */
export function moneyShort(value: number, currency = "USD"): string {
  return Number.isInteger(value) ? moneyWhole(value, currency) : money(value, currency);
}

export function compactMoney(value: number): string {
  if (Math.abs(value) >= 1000) {
    return `$${(value / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  }
  return `$${Math.round(value)}`;
}

export function count(value: number): string {
  return numberFormatter.format(value);
}

export function percent(value: number, digits = 1): string {
  return `${value.toFixed(digits)}%`;
}

export function initials(name: string): string {
  return name
    .split(" ")
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");
}

export function shortDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

export function monthDay(iso: string): string {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
}

export function dayMonth(iso: string): string {
  return new Date(iso).toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

export function dayMonthYear(iso: string): string {
  return new Date(iso).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

export function timeOfDay(iso: string): string {
  return new Date(iso).toLocaleTimeString("en-US", {
    hour: "numeric",
    minute: "2-digit",
  });
}

export function relativeDay(iso: string): string {
  const date = new Date(iso);
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startOfDate = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const diff = Math.round((startOfToday.getTime() - startOfDate.getTime()) / 86_400_000);
  if (diff <= 0) return "Today";
  if (diff === 1) return "Yesterday";
  if (diff < 7) return `${diff} days ago`;
  return monthDay(iso);
}

/** Formats a Unix timestamp in seconds as an ISO string. */
export function fromUnixSeconds(seconds: number): string {
  return new Date(seconds * 1000).toISOString();
}

export function csvEscape(value: string | number | null | undefined): string {
  const text = value === null || value === undefined ? "" : String(value);
  return /[",\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

export function downloadCsv(
  filename: string,
  headers: string[],
  rows: Array<Array<string | number | null | undefined>>,
): void {
  const content = [headers, ...rows].map((row) => row.map(csvEscape).join(",")).join("\n");
  const blob = new Blob([content], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.click();
  URL.revokeObjectURL(url);
}
