import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AiTask, AiTool } from "@fastcart/shared-types";

import { API_BASE_URL } from "@/lib/api";
import {
  aiTaskFilterOptions,
  aiTaskStatusLabel,
  aiTaskStatusTone,
  approveAiTask,
  createAiTask,
  getAiTask,
  listAiTasks,
  listAiTools,
  rejectAiTask,
} from "@/lib/ai-tasks";

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

function requestAt(
  mock: { mock: { calls: unknown[][] } },
  index = 0,
): { url: string; init: RequestInit } {
  const call = mock.mock.calls[index] as [string, RequestInit] | undefined;
  if (!call) throw new Error("fetch was not called");
  return { url: call[0], init: call[1] };
}

const task: AiTask = {
  id: "task_1",
  tool: "catalog.update_price",
  input: { variant_id: "var_1", amount_minor: 1_000 },
  status: "pending",
  error: null,
  created_by: "user_1",
  created_at: 1,
  updated_at: 2,
};

const tool: AiTool = {
  name: "catalog.update_price",
  description: "Set a variant price.",
  input_schema: { type: "object" },
  mutates: true,
  requires_approval: true,
};

beforeEach(() => {
  document.cookie = "fc_csrf=csrf-token";
});

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "fc_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT";
});

describe("AI task client", () => {
  it("lists tools with store, CSRF and credential headers", async () => {
    const mock = vi.fn(async () => jsonResponse([tool]));
    vi.stubGlobal("fetch", mock);

    await expect(listAiTools("store-1")).resolves.toEqual([tool]);

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/ai/tools`);
    expect(init.credentials).toBe("include");
    const headers = init.headers as Headers;
    expect(headers.get("x-store-id")).toBe("store-1");
    expect(headers.get("x-csrf-token")).toBe("csrf-token");
  });

  it("lists tasks with pagination", async () => {
    const page = { data: [task], pagination: { limit: 20, offset: 40, total: 61 } };
    const mock = vi.fn(async () => jsonResponse(page));
    vi.stubGlobal("fetch", mock);

    await expect(listAiTasks("store-1", { limit: 20, offset: 40 })).resolves.toEqual(page);

    const { url } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/ai/tasks?limit=20&offset=40`);
  });

  it("reads one task by encoded id", async () => {
    const mock = vi.fn(async () => jsonResponse(task));
    vi.stubGlobal("fetch", mock);

    await expect(getAiTask("store-1", "task 1")).resolves.toEqual(task);

    const { url } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/ai/tasks/task%201`);
  });

  it("creates a task with the JSON body", async () => {
    const mock = vi.fn(async () => jsonResponse(task, 201));
    vi.stubGlobal("fetch", mock);

    await createAiTask("store-1", {
      tool: "catalog.update_price",
      input: { variant_id: "var_1", amount_minor: 1_000 },
    });

    const { url, init } = requestAt(mock);
    expect(url).toBe(`${API_BASE_URL}/api/v1/admin/ai/tasks`);
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body as string)).toEqual({
      tool: "catalog.update_price",
      input: { variant_id: "var_1", amount_minor: 1_000 },
    });
  });

  it("approves and rejects by encoded id", async () => {
    const mock = vi.fn(async () => jsonResponse(task));
    vi.stubGlobal("fetch", mock);

    await approveAiTask("store-1", "task 1");
    await rejectAiTask("store-1", "task 1");

    const approved = requestAt(mock, 0);
    expect(approved.url).toBe(`${API_BASE_URL}/api/v1/admin/ai/tasks/task%201/approve`);
    expect(approved.init.method).toBe("POST");
    const rejected = requestAt(mock, 1);
    expect(rejected.url).toBe(`${API_BASE_URL}/api/v1/admin/ai/tasks/task%201/reject`);
    expect(rejected.init.method).toBe("POST");
  });

  it("surfaces API error envelopes", async () => {
    const mock = vi.fn(async () =>
      jsonResponse({ error: { message: "illegal AI agent task transition" } }, 409),
    );
    vi.stubGlobal("fetch", mock);

    await expect(approveAiTask("store-1", "task_1")).rejects.toThrow("illegal");
  });
});

describe("AI task helpers", () => {
  it("labels and tones known statuses", () => {
    expect(aiTaskStatusLabel("awaiting_approval")).toBe("Awaiting approval");
    expect(aiTaskStatusLabel("custom")).toBe("custom");
    expect(aiTaskStatusTone("succeeded")).toBe("brand");
    expect(aiTaskStatusTone("failed")).toBe("destructive");
    expect(aiTaskStatusTone("custom")).toBe("secondary");
  });

  it("offers every status in the filter", () => {
    const options = aiTaskFilterOptions();
    expect(options[0]).toEqual({ value: "all", label: "All" });
    expect(options).toHaveLength(7);
    expect(options.map((option) => option.value)).toContain("pending");
  });
});
