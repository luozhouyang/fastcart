import { appendFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { RESULTS_FILE } from "./env.mjs";

export function record(id, status, data = {}) {
  mkdirSync(path.dirname(RESULTS_FILE), { recursive: true });
  appendFileSync(
    RESULTS_FILE,
    `${JSON.stringify({ id, status, at: new Date().toISOString(), ...data })}\n`,
  );
}

export async function gate(id, fn, meta = {}) {
  try {
    const value = await fn();
    record(id, "pass", { ...meta, value });
    return value;
  } catch (error) {
    record(id, "fail", { ...meta, error: String(error) });
    throw error;
  }
}

export async function info(id, fn, meta = {}) {
  try {
    const value = await fn();
    record(id, "info", { ...meta, value });
    return value;
  } catch (error) {
    record(id, "info", { ...meta, error: String(error) });
    return undefined;
  }
}
