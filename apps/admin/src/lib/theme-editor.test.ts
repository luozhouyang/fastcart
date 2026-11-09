import { describe, expect, it } from "vitest";

import {
  buildBlock,
  DEFAULT_THEME_EDITOR_MANIFEST,
  normalizePageBody,
  pageThemeRef,
  paletteGroups,
} from "@/lib/theme-editor";

describe("default theme editor manifest", () => {
  it("describes the generic blocks used by the page editor", () => {
    const types = paletteGroups(DEFAULT_THEME_EDITOR_MANIFEST).flatMap((group) =>
      group.entries.map((entry) => entry.type),
    );
    expect(types).toEqual(
      expect.arrayContaining(["Hero", "RichText", "ProductGrid", "Columns", "GalleryImage"]),
    );
    expect(DEFAULT_THEME_EDITOR_MANIFEST.zones?.header?.allowed_components).toContain(
      "Announcement",
    );
  });

  it("builds blocks from default props and declared slots", () => {
    const hero = buildBlock("Hero", DEFAULT_THEME_EDITOR_MANIFEST, new Set());
    expect(hero.type).toBe("Hero");
    expect(hero.props).toMatchObject({
      id: "hero",
      heading: "Made for everyday rituals",
      align: "left",
    });

    const columns = buildBlock("Columns", DEFAULT_THEME_EDITOR_MANIFEST, new Set(["columns"]));
    expect(columns.props).toMatchObject({ id: "columns-1", gap: "md" });
    expect(Object.keys(columns.slots ?? {})).toEqual(["left", "right"]);
  });

  it("normalizes a page body that has no theme reference", () => {
    const body = normalizePageBody(null);
    expect(body.theme).toBeNull();
    expect(body.content).toEqual([]);
    expect(body.zones).toEqual({});
  });

  it("records theme metadata from a deployment artifact", () => {
    expect(pageThemeRef({ manifest: { id: "atelier", version: "1.4.0" } })).toEqual({
      id: "atelier",
      version: "1.4.0",
    });
    expect(pageThemeRef(null)).toBeNull();
  });
});
