import { readFileSync } from "node:fs";

export function rssKb(pid) {
  try {
    const status = readFileSync(`/proc/${pid}/status`, "utf8");
    const match = status.match(/VmRSS:\s+(\d+)/);
    return match ? Number(match[1]) : null;
  } catch {
    return null;
  }
}
