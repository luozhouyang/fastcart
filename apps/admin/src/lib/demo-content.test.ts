import { describe, expect, it } from "vitest";

import {
  activateDemoTheme,
  deleteDemoTheme,
  getDemoActiveTheme,
  getDemoPage,
  getDemoTheme,
  listDefaultThemeSettingsRevisions,
  listDemoThemes,
  registerDemoTheme,
  rollbackDemoTheme,
  updateDemoPage,
  updateDemoThemeSettings,
} from "@/lib/demo-content";

describe("demo theme artifacts", () => {
  it("seeds themes with the deployment-artifact shape", () => {
    const themes = listDemoThemes();
    expect(themes.length).toBeGreaterThan(0);
    for (const theme of themes) {
      expect(Object.keys(theme.manifest).sort()).toEqual(["id", "name", "version"]);
      expect(Object.keys(theme).sort()).toEqual([
        "created_at",
        "draft_settings",
        "id",
        "is_active",
        "manifest",
        "settings",
        "settings_revision",
        "settings_schema",
        "updated_at",
      ]);
    }
  });

  it("registers a theme from manifest, settings schema and initial settings", () => {
    const theme = registerDemoTheme({
      manifest: { id: "artifact", name: "Artifact", version: "2.0.0" },
      settings_schema: { type: "object" },
      settings: { accent: "#000000" },
    });
    expect(theme.manifest).toEqual({ id: "artifact", name: "Artifact", version: "2.0.0" });
    expect(theme.settings_schema).toEqual({ type: "object" });
    expect(theme.settings).toEqual({ accent: "#000000" });
    expect(getDemoTheme(theme.id)?.manifest.id).toBe("artifact");
  });

  it("activates, rolls back and records settings revisions", () => {
    const initial = getDemoActiveTheme();
    expect(initial).not.toBeNull();
    const target = listDemoThemes().find((theme) => !theme.is_active);
    expect(target).toBeDefined();
    if (!target) return;

    activateDemoTheme(target.id);
    expect(getDemoActiveTheme()?.id).toBe(target.id);
    rollbackDemoTheme();
    expect(getDemoActiveTheme()).not.toBeNull();
    expect(getDemoActiveTheme()?.id).not.toBe(target.id);

    const theme = listDemoThemes()[0];
    expect(theme).toBeDefined();
    if (!theme) return;
    const updated = updateDemoThemeSettings(theme.id, { site_name: "Updated" });
    expect(updated.settings).toEqual({ site_name: "Updated" });
    expect(updated.settings_revision).toBe(theme.settings_revision + 1);
    expect(listDefaultThemeSettingsRevisions(theme.id)[0]?.revision).toBe(
      theme.settings_revision + 1,
    );
  });

  it("refuses to delete the active theme", () => {
    const active = getDemoActiveTheme();
    expect(active).not.toBeNull();
    if (!active) return;
    expect(() => deleteDemoTheme(active.id)).toThrow("The active theme cannot be deleted");
  });
});

describe("demo page editing", () => {
  it("saves page bodies with optimistic revisions", () => {
    const page = getDemoPage("page_home");
    expect(page).not.toBeNull();
    if (!page) return;

    const updated = updateDemoPage("page_home", { content: [] }, page.revision);
    expect(updated.revision).toBe(page.revision + 1);
    expect(updated.body).toEqual({ content: [] });
    expect(() => updateDemoPage("page_home", { content: [] }, page.revision)).toThrow(
      "This page changed elsewhere. Reload and try again.",
    );
  });
});
