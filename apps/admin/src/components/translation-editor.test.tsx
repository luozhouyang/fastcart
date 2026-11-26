import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { TranslationEditor } from "@/components/translation-editor";
import { AuthProvider } from "@/lib/auth";
import { DEMO_STORAGE_KEY } from "@/lib/api";

const FIELDS = [
  { key: "title", label: "Title" },
  { key: "description", label: "Description", multiline: true },
];

function renderEditor() {
  return render(
    <AuthProvider>
      <TranslationEditor
        resourceType="product"
        resourceId="prd_demo_1"
        fields={FIELDS}
        source={{ title: "Ridge Ceramic Carafe", description: "Hand-thrown stoneware." }}
      />
    </AuthProvider>,
  );
}

beforeEach(() => {
  window.sessionStorage.setItem(DEMO_STORAGE_KEY, "1");
});

afterEach(() => {
  window.sessionStorage.removeItem(DEMO_STORAGE_KEY);
});

describe("translation editor in demo mode", () => {
  it("starts untranslated and defaults to the first non-default locale", async () => {
    renderEditor();

    expect(await screen.findByText(/Untranslated ·/)).toBeInTheDocument();
    expect(screen.getByLabelText("Locale")).toHaveValue("fr-FR");
    expect(screen.getByText("0/2 fields translated")).toBeInTheDocument();
    // The source content is shown as the fallback placeholder.
    expect(screen.getByLabelText("Title")).toHaveAttribute("placeholder", "Ridge Ceramic Carafe");
  });

  it("saves a translation and re-reads it", async () => {
    const user = userEvent.setup();
    renderEditor();

    const title = await screen.findByLabelText("Title");
    await user.type(title, "Carafe en céramique");
    await user.type(screen.getByLabelText("Description"), "Grès tourné à la main.");
    await user.click(screen.getByRole("button", { name: "Save translation" }));

    expect(await screen.findByText("2/2 fields translated")).toBeInTheDocument();
    expect(screen.getByText(/Editing the/)).toBeInTheDocument();
    expect(screen.queryByText(/Untranslated ·/)).not.toBeInTheDocument();
    expect(screen.getByLabelText("Title")).toHaveValue("Carafe en céramique");
    expect(screen.getByLabelText("Description")).toHaveValue("Grès tourné à la main.");

    // The preview overlays the draft on the source content.
    const preview = screen.getByText("Storefront preview").parentElement;
    if (!preview) throw new Error("expected a storefront preview");
    expect(within(preview).getByText("Carafe en céramique")).toBeInTheDocument();
    expect(within(preview).getByText("Grès tourné à la main.")).toBeInTheDocument();

    // Switching away and back keeps the stored row.
    await user.selectOptions(screen.getByLabelText("Locale"), "de-DE");
    expect(await screen.findByText(/Untranslated ·/)).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("Locale"), "fr-FR");
    expect(await screen.findByDisplayValue("Carafe en céramique")).toBeInTheDocument();
  });

  it("keeps each locale translation separate", async () => {
    const user = userEvent.setup();
    renderEditor();

    await user.type(await screen.findByLabelText("Title"), "Carafe");
    await user.click(screen.getByRole("button", { name: "Save translation" }));

    await user.selectOptions(screen.getByLabelText("Locale"), "de-DE");
    await user.type(await screen.findByLabelText("Title"), "Karaffe");
    await user.click(screen.getByRole("button", { name: "Save translation" }));
    expect(await screen.findByText("1/2 fields translated")).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("Locale"), "fr-FR");
    expect(await screen.findByDisplayValue("Carafe")).toBeInTheDocument();
  });

  it("deletes a saved translation back to the untranslated state", async () => {
    const user = userEvent.setup();
    renderEditor();

    await user.type(await screen.findByLabelText("Title"), "Carafe");
    await user.click(screen.getByRole("button", { name: "Save translation" }));
    expect(await screen.findByDisplayValue("Carafe")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete translation" }));

    expect(await screen.findByText(/Untranslated ·/)).toBeInTheDocument();
    expect(screen.getByLabelText("Title")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Delete translation" })).toBeDisabled();
  });

  it("omits blank fields so the storefront falls back to the source", async () => {
    const user = userEvent.setup();
    renderEditor();

    await user.type(await screen.findByLabelText("Title"), "Carafe");
    await user.click(screen.getByRole("button", { name: "Save translation" }));

    const preview = (await screen.findByText("Storefront preview")).parentElement;
    if (!preview) throw new Error("expected a storefront preview");
    expect(within(preview).getByText("Carafe")).toBeInTheDocument();
    // The untranslated description keeps the source copy in the preview.
    expect(within(preview).getByText("Hand-thrown stoneware.")).toBeInTheDocument();
  });

  it("disables saving until something is typed", async () => {
    renderEditor();

    expect(await screen.findByRole("button", { name: "Save translation" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete translation" })).toBeDisabled();
  });
});
