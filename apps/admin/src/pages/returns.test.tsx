import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ReturnsPage } from "@/pages/returns";
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

describe("returns page in demo mode", () => {
  it("lists the fixtures with order context and links each order", async () => {
    renderDemo(<ReturnsPage />);

    expect(await screen.findByText("Returns")).toBeInTheDocument();
    const table = screen.getByRole("table");
    expect(within(table).getByText("#1047")).toBeInTheDocument();
    expect(within(table).getByText("imogen@halcyonsupply.test")).toBeInTheDocument();
    expect(within(table).getAllByText("Arrived damaged").length).toBeGreaterThan(0);

    // Order numbers link back to the orders page with the order selected.
    const link = within(table).getByRole("link", { name: "#1047" });
    expect(link).toHaveAttribute("href", "/orders?order=order_demo_1047");
  });

  it("filters by status", async () => {
    const user = userEvent.setup();
    renderDemo(<ReturnsPage />);

    expect(await screen.findByText(/6 returns/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Requested" }));

    expect(await screen.findByText(/1 return\b/)).toBeInTheDocument();
    const table = screen.getByRole("table");
    expect(within(table).getByText("imogen@halcyonsupply.test")).toBeInTheDocument();
    expect(within(table).queryByText("dara@halcyonsupply.test")).not.toBeInTheDocument();
    expect(within(table).queryByText("Written off")).not.toBeInTheDocument();
    expect(within(table).getAllByRole("row")).toHaveLength(2);
  });

  it("resets the status filter back to all", async () => {
    const user = userEvent.setup();
    renderDemo(<ReturnsPage />);

    await screen.findByText(/6 returns/);
    await user.click(screen.getByRole("button", { name: "Requested" }));
    await screen.findByText(/1 return\b/);

    const group = screen.getByRole("group", { name: "Filter returns by status" });
    expect(within(group).getByRole("button", { name: "Requested" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await user.click(within(group).getByRole("button", { name: "All" }));
    expect(await screen.findByText(/6 returns/)).toBeInTheDocument();
    expect(screen.getByRole("table")).toHaveTextContent("dara@halcyonsupply.test");
  });
});
