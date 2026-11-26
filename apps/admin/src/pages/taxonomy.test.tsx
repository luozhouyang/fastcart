import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { TaxonomyPage } from "@/pages/taxonomy";
import { AuthProvider } from "@/lib/auth";
import { DEMO_STORAGE_KEY } from "@/lib/api";

function renderDemo() {
  return render(
    <AuthProvider>
      <TaxonomyPage />
    </AuthProvider>,
  );
}

beforeEach(() => {
  window.sessionStorage.setItem(DEMO_STORAGE_KEY, "1");
});

afterEach(() => {
  window.sessionStorage.removeItem(DEMO_STORAGE_KEY);
});

describe("taxonomy page in demo mode", () => {
  it("renders the category tree and collection fixtures", async () => {
    const user = userEvent.setup();
    renderDemo();

    expect(await screen.findByText("Taxonomy")).toBeInTheDocument();
    expect(screen.getByText("Home")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: /Collections/ }));
    expect(screen.getByText("New arrivals")).toBeInTheDocument();
  });

  it("lists product types and creates a new one", async () => {
    const user = userEvent.setup();
    renderDemo();

    await user.click(await screen.findByRole("tab", { name: /Product types/ }));
    expect(screen.getByText("Shirt")).toBeInTheDocument();
    expect(screen.getByText("Digital download")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "New product type" }));
    const value = await screen.findByLabelText("Value");
    await user.type(value, "Hat");
    await user.click(screen.getByRole("button", { name: "Create product type" }));

    expect(await screen.findByText("Hat")).toBeInTheDocument();
  });

  it("renames a product type from its row menu", async () => {
    const user = userEvent.setup();
    renderDemo();

    await user.click(await screen.findByRole("tab", { name: /Product types/ }));
    await user.click(screen.getByRole("button", { name: "Actions for Shirt" }));
    await user.click(await screen.findByRole("menuitem", { name: "Rename type" }));

    const value = await screen.findByLabelText("Value");
    await user.clear(value);
    await user.type(value, "T-shirt");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByText("T-shirt")).toBeInTheDocument();
    expect(screen.queryByText("Shirt")).not.toBeInTheDocument();
  });

  it("deletes a product type after confirmation", async () => {
    const user = userEvent.setup();
    renderDemo();

    await user.click(await screen.findByRole("tab", { name: /Product types/ }));
    await user.click(screen.getByRole("button", { name: "Actions for Mug" }));
    await user.click(await screen.findByRole("menuitem", { name: "Delete" }));
    await user.click(await screen.findByRole("button", { name: "Delete" }));

    expect(await screen.findByText("Shirt")).toBeInTheDocument();
    expect(screen.queryByText("Mug")).not.toBeInTheDocument();
  });
});
