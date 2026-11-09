import type {
  CreatePageRequest,
  JsonValue,
  Page,
  PageBody,
  PageVersion,
  PreviewTokenResponse,
  RegisterThemeRequest,
  Theme,
  ThemeSettingsRevision,
} from "@fastcart/shared-types";

const now = () => Math.floor(Date.now() / 1000);

function seedTheme(id: string, name: string, version: string, isActive: boolean): Theme {
  const created = now() - 86_400 * 14;
  return {
    id,
    manifest: { id, name, version },
    settings_schema: {
      type: "object",
      properties: {
        site_name: { type: "string" },
        accent: { type: "string" },
        show_announcement: { type: "boolean" },
      },
    },
    settings: {
      site_name: name,
      accent: "#1c6b52",
      show_announcement: true,
    },
    draft_settings: {
      site_name: name,
      accent: "#1c6b52",
      show_announcement: true,
    },
    settings_revision: 3,
    is_active: isActive,
    created_at: created,
    updated_at: created + 90,
  };
}

let themes: Theme[] = [
  seedTheme("theme_atelier", "Atelier", "1.4.0", true),
  seedTheme("theme_minimal", "Minimal", "0.9.1", false),
];

let themeSettingsRevisions: Record<string, ThemeSettingsRevision[]> = {
  theme_atelier: [
    {
      id: "tsr_atelier_3",
      theme_id: "theme_atelier",
      revision: 3,
      settings: { site_name: "Atelier", accent: "#1c6b52", show_announcement: true },
      status: "draft",
      created_at: now() - 3_600,
    },
    {
      id: "tsr_atelier_2",
      theme_id: "theme_atelier",
      revision: 2,
      settings: { site_name: "Atelier", accent: "#b45309", show_announcement: true },
      status: "published",
      created_at: now() - 86_400,
    },
  ],
};

const homeBody: PageBody = {
  schema_version: 1,
  theme: { id: "atelier", version: "1.4.0", schema_version: 1 },
  content: [
    {
      type: "Hero",
      props: {
        id: "hero-1",
        heading: "Made for everyday rituals",
        subheading: "Thoughtful objects for a quieter home.",
        align: "left",
        overlay: false,
      },
      slots: {},
    },
    {
      type: "ProductGrid",
      props: {
        id: "grid-1",
        collection: { source: "collection", id: "col_new" },
        limit: 8,
        columns: "3",
      },
      slots: {},
    },
    {
      type: "Columns",
      props: { id: "columns-1", gap: "md" },
      slots: {
        left: [
          {
            type: "RichText",
            props: { id: "rich-1", body: "<p>Our story, in objects.</p>", width: "narrow" },
            slots: {},
          },
        ],
        right: [],
      },
    },
  ],
  root: { props: { page_title: "Home" } },
  zones: {},
};

const aboutBody: PageBody = {
  schema_version: 1,
  theme: { id: "atelier", version: "1.4.0", schema_version: 1 },
  content: [
    {
      type: "RichText",
      props: { id: "rich-about", body: "<p>We make things that last.</p>", width: "wide" },
      slots: {},
    },
  ],
  root: { props: { page_title: "About" } },
  zones: {},
};

let pages: Page[] = [
  {
    id: "page_home",
    title: "Home",
    handle: "home",
    status: "published",
    page_type: "home",
    revision: 2,
    body: homeBody as unknown as JsonValue,
    published_body: homeBody as unknown as JsonValue,
    seo: {},
    metadata: {},
    published_at: now() - 86_400,
    created_at: now() - 86_400 * 30,
    updated_at: now() - 3_600,
    deleted_at: null,
  },
  {
    id: "page_about",
    title: "About",
    handle: "about",
    status: "draft",
    page_type: "custom",
    revision: 1,
    body: aboutBody as unknown as JsonValue,
    seo: {},
    metadata: {},
    published_at: null,
    created_at: now() - 86_400 * 7,
    updated_at: now() - 86_400,
    deleted_at: null,
  },
];

let pageVersions: Record<string, PageVersion[]> = {
  page_home: [
    {
      id: "pv_home_2",
      page_id: "page_home",
      version: 2,
      body: homeBody as unknown as JsonValue,
      kind: "published",
      origin: "manual",
      created_by: "demo",
      created_at: now() - 3_600,
    },
    {
      id: "pv_home_1",
      page_id: "page_home",
      version: 1,
      body: aboutBody as unknown as JsonValue,
      kind: "draft",
      origin: "manual",
      created_by: "demo",
      created_at: now() - 86_400,
    },
  ],
  page_about: [
    {
      id: "pv_about_1",
      page_id: "page_about",
      version: 1,
      body: aboutBody as unknown as JsonValue,
      kind: "draft",
      origin: "manual",
      created_by: "demo",
      created_at: now() - 86_400,
    },
  ],
};

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function slug(value: string): string {
  return (
    value
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || `page-${now()}`
  );
}

export function listDemoThemes(): Theme[] {
  return clone(themes);
}

export function getDemoTheme(id: string): Theme | null {
  const theme = themes.find((entry) => entry.id === id);
  return theme ? clone(theme) : null;
}

export function getDemoActiveTheme(): Theme | null {
  const theme = themes.find((entry) => entry.is_active);
  return theme ? clone(theme) : null;
}

export function registerDemoTheme(input: RegisterThemeRequest): Theme {
  const theme = seedTheme(
    `theme_${input.manifest.id}_${now()}`,
    input.manifest.name,
    input.manifest.version,
    false,
  );
  theme.manifest = clone(input.manifest);
  theme.settings_schema = clone(input.settings_schema ?? theme.settings_schema);
  theme.settings = clone(input.settings ?? {});
  theme.draft_settings = clone(input.settings ?? {});
  themes = [...themes, theme];
  return clone(theme);
}

export function activateDemoTheme(id: string): Theme {
  const theme = themes.find((entry) => entry.id === id);
  if (!theme) throw new Error("Theme not found");
  themes = themes.map((entry) => ({ ...entry, is_active: entry.id === id }));
  return clone(theme);
}

export function rollbackDemoTheme(): Theme {
  const active = themes.find((entry) => entry.is_active);
  const target = [...themes].reverse().find((entry) => entry.id !== active?.id);
  if (!target) throw new Error("No previous theme to roll back to");
  themes = themes.map((entry) => ({
    ...entry,
    is_active: entry.id === target.id,
  }));
  return clone(target);
}

export function deleteDemoTheme(id: string): void {
  const theme = themes.find((entry) => entry.id === id);
  if (theme?.is_active) throw new Error("The active theme cannot be deleted");
  themes = themes.filter((entry) => entry.id !== id);
}

export function updateDemoThemeSettings(id: string, settings: JsonValue): Theme {
  const theme = themes.find((entry) => entry.id === id);
  if (!theme) throw new Error("Theme not found");
  const revision = (theme.settings_revision ?? 0) + 1;
  theme.settings = clone(settings);
  theme.draft_settings = clone(settings);
  theme.settings_revision = revision;
  theme.updated_at = now();
  themeSettingsRevisions = {
    ...themeSettingsRevisions,
    [id]: [
      {
        id: `tsr_${id}_${revision}`,
        theme_id: id,
        revision,
        settings: clone(settings),
        status: "draft",
        created_at: now(),
      },
      ...(themeSettingsRevisions[id] ?? []),
    ],
  };
  return clone(theme);
}

export function listDefaultThemeSettingsRevisions(id: string): ThemeSettingsRevision[] {
  return clone(themeSettingsRevisions[id] ?? []);
}

export function listDemoPages(): Page[] {
  return clone(pages).filter((page) => page.deleted_at === null);
}

export function getDemoPage(id: string): Page | null {
  const page = pages.find((entry) => entry.id === id);
  return page ? clone(page) : null;
}

export function createDemoPage(input: CreatePageRequest): Page {
  const timestamp = now();
  const page: Page = {
    id: `page_${timestamp}_${pages.length + 1}`,
    title: input.title,
    handle: input.handle?.trim() || slug(input.title),
    status: "draft",
    page_type: input.page_type ?? "custom",
    revision: 1,
    body: clone(input.body ?? {}),
    seo: clone(input.seo ?? {}),
    metadata: clone(input.metadata ?? {}),
    published_at: null,
    created_at: timestamp,
    updated_at: timestamp,
    deleted_at: null,
  };
  pages = [page, ...pages];
  pageVersions = {
    ...pageVersions,
    [page.id]: [
      {
        id: `pv_${page.id}_1`,
        page_id: page.id,
        version: 1,
        body: clone(page.body),
        kind: "draft",
        origin: "manual",
        created_by: "demo",
        created_at: timestamp,
      },
    ],
  };
  return clone(page);
}

export function updateDemoPage(id: string, body: JsonValue, baseRevision: number): Page {
  const page = pages.find((entry) => entry.id === id);
  if (!page) throw new Error("Page not found");
  if (page.revision !== baseRevision) {
    throw new Error("This page changed elsewhere. Reload and try again.");
  }
  const revision = page.revision + 1;
  page.body = clone(body);
  page.revision = revision;
  page.updated_at = now();
  pageVersions = {
    ...pageVersions,
    [id]: [
      {
        id: `pv_${id}_${revision}`,
        page_id: id,
        version: revision,
        body: clone(body),
        kind: "draft",
        origin: "manual",
        created_by: "demo",
        created_at: now(),
      },
      ...(pageVersions[id] ?? []),
    ],
  };
  return clone(page);
}

export function publishDemoPage(id: string): Page {
  const page = pages.find((entry) => entry.id === id);
  if (!page) throw new Error("Page not found");
  page.status = "published";
  page.published_body = clone(page.body);
  page.published_at = now();
  page.updated_at = now();
  return clone(page);
}

export function unpublishDemoPage(id: string): Page {
  const page = pages.find((entry) => entry.id === id);
  if (!page) throw new Error("Page not found");
  page.status = "draft";
  page.published_at = null;
  page.updated_at = now();
  return clone(page);
}

export function archiveDemoPage(id: string): Page {
  const page = pages.find((entry) => entry.id === id);
  if (!page) throw new Error("Page not found");
  page.status = "archived";
  page.updated_at = now();
  return clone(page);
}

export function listDefaultPageVersions(id: string): PageVersion[] {
  return clone(pageVersions[id] ?? []);
}

export function restoreDemoPageVersion(id: string, version: number): Page {
  const page = pages.find((entry) => entry.id === id);
  if (!page) throw new Error("Page not found");
  const snapshot = (pageVersions[id] ?? []).find((entry) => entry.version === version);
  if (!snapshot) throw new Error("Version not found");
  page.body = clone(snapshot.body);
  page.revision += 1;
  page.updated_at = now();
  return clone(page);
}

export function issueDemoPreviewToken(pageId: string): PreviewTokenResponse {
  return {
    token: `demo.${pageId}.${now()}`,
    expires_at: now() + 15 * 60,
    jti: `demo-${now()}`,
  };
}
