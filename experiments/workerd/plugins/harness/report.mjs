import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

import { REPORT_DIR, RESULTS_FILE, WORKERD } from "./env.mjs";

const rows = existsSync(RESULTS_FILE)
  ? readFileSync(RESULTS_FILE, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line))
  : [];

const latest = new Map();
for (const row of rows) latest.set(row.id, row);
const entries = [...latest.values()].sort((a, b) =>
  a.id.localeCompare(b.id, undefined, { numeric: true }),
);

const version = execFileSync(WORKERD, ["--version"], { encoding: "utf8" }).trim();
const gates = entries.filter((entry) => entry.status === "pass" || entry.status === "fail");
const failed = gates.filter((entry) => entry.status === "fail");
const info = entries.filter((entry) => entry.status === "info");

const lines = [
  "# Plugin P0 Report",
  "",
  `Generated: ${new Date().toISOString()}`,
  "",
  `- workerd: \`${version}\``,
  `- node: \`${process.version}\``,
  `- gates passed: ${gates.filter((entry) => entry.status === "pass").length}/${gates.length}`,
  `- informational: ${info.length}`,
  "",
  "## Gates",
  "",
  "| ID | Status | Value |",
  "| --- | --- | --- |",
  ...gates.map((entry) => `| ${entry.id} | ${entry.status} | ${cell(entry)} |`),
  "",
  "## Informational",
  "",
  "| ID | Value |",
  "| --- | --- |",
  ...info.map((entry) => `| ${entry.id} | ${cell(entry)} |`),
  "",
];

function cell(entry) {
  const value = entry.error ?? entry.value ?? "";
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.replaceAll("|", "\\|").slice(0, 300);
}

mkdirSync(REPORT_DIR, { recursive: true });
writeFileSync(path.join(REPORT_DIR, "report.md"), lines.join("\n"));
writeFileSync(path.join(REPORT_DIR, "summary.json"), JSON.stringify({ version, entries }, null, 2));
console.log(`report: ${path.join(REPORT_DIR, "report.md")}`);
if (failed.length > 0) {
  console.error(`failed gates: ${failed.map((entry) => entry.id).join(", ")}`);
  process.exitCode = 1;
}
