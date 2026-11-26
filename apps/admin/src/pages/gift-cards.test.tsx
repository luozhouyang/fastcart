import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { GiftCardsPage } from "@/pages/gift-cards";
import { ReturnReasonsPage } from "@/pages/return-reasons";
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

describe("gift cards page in demo mode", () => {
  it("lists the fixtures with balances", async () => {
    renderDemo(<GiftCardsPage />);

    expect(await screen.findByText("Gift cards")).toBeInTheDocument();
    expect(screen.getByText("7QK2M9XR4TZB1HVN")).toBeInTheDocument();
    expect(screen.getByText("HALCYON-FRIENDS")).toBeInTheDocument();
  });

  it("issues a card and keeps the dialog validation", async () => {
    const user = userEvent.setup();
    renderDemo(<GiftCardsPage />);

    await user.click(await screen.findByRole("button", { name: "Issue gift card" }));
    await user.click(await screen.findByRole("button", { name: "Issue card" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(/greater than zero/);

    const amount = screen.getByLabelText("Amount");
    await user.type(amount, "50");
    await user.click(screen.getByRole("button", { name: "Issue card" }));

    // The demo issues a card with a generated placeholder code.
    expect(await screen.findByText("DEMO0000DEMO0000")).toBeInTheDocument();
  });

  it("disables an active card", async () => {
    const user = userEvent.setup();
    renderDemo(<GiftCardsPage />);

    const buttons = await screen.findAllByRole("button", { name: "Disable" });
    const first = buttons[0];
    if (!first) throw new Error("expected a disable button");
    await user.click(first);

    // The toggled row now offers to enable the card again; the disabled
    // fixture row already did, so two such buttons exist.
    expect(await screen.findAllByRole("button", { name: "Enable" })).toHaveLength(2);
  });
});

describe("return reasons page in demo mode", () => {
  it("lists the fixtures and creates a reason", async () => {
    const user = userEvent.setup();
    renderDemo(<ReturnReasonsPage />);

    expect(await screen.findByText("Return reasons")).toBeInTheDocument();
    expect(screen.getByText("Arrived damaged")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New reason" }));
    await user.type(await screen.findByLabelText("Value"), "too_small");
    await user.type(screen.getByLabelText("Label"), "Too small");
    await user.click(screen.getByRole("button", { name: "Create reason" }));

    expect(await screen.findByText("Too small")).toBeInTheDocument();
  });

  it("requires both the value and the label", async () => {
    const user = userEvent.setup();
    renderDemo(<ReturnReasonsPage />);

    await user.click(await screen.findByRole("button", { name: "New reason" }));
    await user.click(screen.getByRole("button", { name: "Create reason" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(/value/);
  });
});
