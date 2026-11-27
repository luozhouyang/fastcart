import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { AiTasksPage } from "@/pages/ai-tasks";
import { AuthProvider } from "@/lib/auth";
import { DEMO_STORAGE_KEY } from "@/lib/api";

function renderDemo(ui: React.ReactElement) {
  return render(<AuthProvider>{ui}</AuthProvider>);
}

beforeEach(() => {
  window.sessionStorage.setItem(DEMO_STORAGE_KEY, "1");
});

afterEach(() => {
  window.sessionStorage.removeItem(DEMO_STORAGE_KEY);
});

describe("AI tasks page in demo mode", () => {
  it("renders the tool catalog and the task list", async () => {
    const user = userEvent.setup();
    renderDemo(<AiTasksPage />);

    expect(await screen.findByText("AI tasks")).toBeInTheDocument();
    expect(screen.getAllByText("catalog.update_price").length).toBeGreaterThan(0);
    expect(screen.getByText("Requires approval")).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Show input schema for catalog.update_price" }),
    );
    expect(await screen.findByText(/"amount_minor"/)).toBeInTheDocument();

    const table = screen.getByRole("table");
    expect(within(table).getByText("Succeeded")).toBeInTheDocument();
    expect(within(table).getByText("Pending")).toBeInTheDocument();
    expect(within(table).getAllByText("user_demo_owner").length).toBeGreaterThan(0);
  });

  it("approves a pending task after confirming", async () => {
    const user = userEvent.setup();
    renderDemo(<AiTasksPage />);

    await user.click(await screen.findByRole("button", { name: "Open task task_demo_9003" }));
    const sheet = await screen.findByRole("dialog", { name: "task_demo_9003" });
    expect(within(sheet).getByText("Pending")).toBeInTheDocument();

    await user.click(within(sheet).getByRole("button", { name: "Approve" }));
    const confirm = await screen.findByRole("dialog", { name: "Approve this task?" });
    await user.click(within(confirm).getByRole("button", { name: "Run task" }));

    expect(await within(sheet).findByText("Running")).toBeInTheDocument();
  });

  it("rejects a pending task without confirming", async () => {
    const user = userEvent.setup();
    renderDemo(<AiTasksPage />);

    await user.click(await screen.findByRole("button", { name: "Open task task_demo_9003" }));
    const sheet = await screen.findByRole("dialog", { name: "task_demo_9003" });

    await user.click(within(sheet).getByRole("button", { name: "Reject" }));

    expect(await within(sheet).findByText("Cancelled")).toBeInTheDocument();
    expect(within(sheet).queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
  });

  it("shows the output of a succeeded task only when it is present", async () => {
    const user = userEvent.setup();
    renderDemo(<AiTasksPage />);

    await user.click(await screen.findByRole("button", { name: "Open task task_demo_9001" }));
    const sheet = await screen.findByRole("dialog", { name: "task_demo_9001" });
    expect(within(sheet).getByText("Output")).toBeInTheDocument();
    expect(within(sheet).getByText(/"available": 42/)).toBeInTheDocument();

    await user.click(within(sheet).getByRole("button", { name: "Close" }));
    await user.click(await screen.findByRole("button", { name: "Open task task_demo_9003" }));
    const pendingSheet = await screen.findByRole("dialog", { name: "task_demo_9003" });
    expect(within(pendingSheet).queryByText("Output")).not.toBeInTheDocument();
  });
});
