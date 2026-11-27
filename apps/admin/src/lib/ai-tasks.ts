import type { AiTask, AiTool, CreateAiTaskRequest, PageResult } from "@fastcart/shared-types";

import { API_BASE_URL, apiErrorMessage, readCookie } from "@/lib/api";

/** One page of recorded agent tasks. */
export type AiTaskPage = PageResult<AiTask>;

/** Lifecycle states the agent layer records for a task. */
export type AiTaskStatus =
  "pending" | "running" | "awaiting_approval" | "succeeded" | "failed" | "cancelled";

/** Query accepted by the task list. */
export interface AiTaskQuery {
  limit?: number;
  offset?: number;
}

export const AI_TASK_STATUSES: readonly AiTaskStatus[] = [
  "pending",
  "running",
  "awaiting_approval",
  "succeeded",
  "failed",
  "cancelled",
];

export const AI_TASK_STATUS_LABELS: Record<AiTaskStatus, string> = {
  pending: "Pending",
  running: "Running",
  awaiting_approval: "Awaiting approval",
  succeeded: "Succeeded",
  failed: "Failed",
  cancelled: "Cancelled",
};

export type AiTaskStatusTone = "brand" | "warning" | "info" | "destructive" | "secondary";

export const AI_TASK_STATUS_TONES: Record<AiTaskStatus, AiTaskStatusTone> = {
  pending: "warning",
  running: "info",
  awaiting_approval: "warning",
  succeeded: "brand",
  failed: "destructive",
  cancelled: "secondary",
};

/** Merchant-facing status label; unknown values fall back to the raw value. */
export function aiTaskStatusLabel(status: string): string {
  return AI_TASK_STATUS_LABELS[status as AiTaskStatus] ?? status;
}

/** Badge tone for a status, defaulting to secondary for unknown values. */
export function aiTaskStatusTone(status: string): AiTaskStatusTone {
  return AI_TASK_STATUS_TONES[status as AiTaskStatus] ?? "secondary";
}

/** Status filter options for the task table; `all` clears the filter. */
export function aiTaskFilterOptions(): Array<{
  value: AiTaskStatus | "all";
  label: string;
}> {
  return [
    { value: "all", label: "All" },
    ...AI_TASK_STATUSES.map((status) => ({
      value: status,
      label: AI_TASK_STATUS_LABELS[status],
    })),
  ];
}

async function aiRequest<T>(storeId: string, path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set("x-store-id", storeId);
  const csrf = readCookie("fc_csrf");
  if (csrf) headers.set("x-csrf-token", csrf);
  const response = await fetch(`${API_BASE_URL}/api/v1/admin${path}`, {
    ...init,
    credentials: "include",
    headers,
  });
  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new Error(apiErrorMessage(body, `AI request failed (${response.status})`));
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

/** Lists every tool registered with the agent layer. */
export function listAiTools(storeId: string, signal?: AbortSignal): Promise<AiTool[]> {
  return aiRequest<AiTool[]>(storeId, "/ai/tools", signal ? { signal } : {});
}

/** Lists one page of recorded tasks, newest first. */
export function listAiTasks(
  storeId: string,
  query: AiTaskQuery = {},
  signal?: AbortSignal,
): Promise<AiTaskPage> {
  const params = new URLSearchParams({
    limit: String(query.limit ?? 25),
    offset: String(query.offset ?? 0),
  });
  return aiRequest<AiTaskPage>(storeId, `/ai/tasks?${params.toString()}`, signal ? { signal } : {});
}

/** Reads one recorded task. */
export function getAiTask(storeId: string, taskId: string, signal?: AbortSignal): Promise<AiTask> {
  return aiRequest<AiTask>(
    storeId,
    `/ai/tasks/${encodeURIComponent(taskId)}`,
    signal ? { signal } : {},
  );
}

/** Queues a tool invocation for review or execution. */
export function createAiTask(
  storeId: string,
  input: CreateAiTaskRequest,
  signal?: AbortSignal,
): Promise<AiTask> {
  return aiRequest<AiTask>(storeId, "/ai/tasks", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    ...(signal ? { signal } : {}),
  });
}

/** Approves a pending task, which lets it run. */
export function approveAiTask(
  storeId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<AiTask> {
  return aiRequest<AiTask>(storeId, `/ai/tasks/${encodeURIComponent(taskId)}/approve`, {
    method: "POST",
    ...(signal ? { signal } : {}),
  });
}

/** Rejects a pending task without running it. */
export function rejectAiTask(
  storeId: string,
  taskId: string,
  signal?: AbortSignal,
): Promise<AiTask> {
  return aiRequest<AiTask>(storeId, `/ai/tasks/${encodeURIComponent(taskId)}/reject`, {
    method: "POST",
    ...(signal ? { signal } : {}),
  });
}
