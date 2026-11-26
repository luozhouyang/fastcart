import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { CampaignsPage } from "@/pages/campaigns";
import { PaymentsPage } from "@/pages/payments";
import { PriceListsPage } from "@/pages/price-lists";
import { WebhooksPage } from "@/pages/webhooks";
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

describe("commerce pages in demo mode", () => {
  it("renders the price lists fixture and expands its prices", async () => {
    const user = userEvent.setup();
    renderDemo(<PriceListsPage />);

    expect(await screen.findByText("Price lists")).toBeInTheDocument();
    expect(screen.getByText("Wholesale spring")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Expand Wholesale spring" }));
    expect(await screen.findByText("var_ridge_sage")).toBeInTheDocument();
  });

  it("renders the campaign fixture", async () => {
    renderDemo(<CampaignsPage />);

    expect(await screen.findByText("Campaigns")).toBeInTheDocument();
    expect(screen.getByText("Autumn studio edit")).toBeInTheDocument();
    expect(screen.getByText("$785.00 of $2,500.00")).toBeInTheDocument();
  });

  it("renders webhook endpoints without a server", async () => {
    renderDemo(<WebhooksPage />);

    expect(await screen.findByText("Webhooks")).toBeInTheDocument();
    expect(
      screen.getByText("https://hooks.halcyonsupply.test/fastcart/orders"),
    ).toBeInTheDocument();
    expect(screen.getByText("order.placed +2")).toBeInTheDocument();
  });

  it("renders payments and opens the detail sheet", async () => {
    const user = userEvent.setup();
    renderDemo(<PaymentsPage />);

    expect(await screen.findByText("Payments")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Open payment pay_demo_1041" }));

    expect(await screen.findByText("Payment session")).toBeInTheDocument();
    expect(screen.getByText("Chipped lid")).toBeInTheDocument();
  });
});
