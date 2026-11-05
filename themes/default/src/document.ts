/** Escapes text inserted into HTML text or attribute positions. */
export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Document inputs assembled by the server renderer. */
export interface DocumentShellOptions {
  lang: string;
  title: string;
  seo: unknown;
  settings: Record<string, unknown>;
  /** Pre-serialized hydration state. */
  hydrationJson: string;
  /** Browser module URL, or null when no public asset base was provided. */
  clientScriptUrl: string | null;
}

/** Document inputs including the already rendered page tree. */
export interface DocumentOptions extends DocumentShellOptions {
  bodyHtml: string;
}

/**
 * Builds the part of the document that can be sent before React starts.
 *
 * Streaming uses this at shell time so the browser gets the head, styles and
 * the empty root while the page tree is still rendering.
 */
export function renderDocumentPrefix(options: DocumentShellOptions): string {
  const title = options.title.trim() || "Fastcart";
  const lang = options.lang.trim() || "en";
  const seoHead = renderSeoHead(options.seo);
  const style = renderThemeStyle(options.settings);

  return `<!doctype html>
<html lang="${escapeHtml(lang)}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title>
${seoHead}
<style>${style}</style>
</head>
<body>
<div id="fastcart-root">`;
}

/** Builds the closing markup, hydration state and client script. */
export function renderDocumentSuffix(options: DocumentShellOptions): string {
  const script = options.clientScriptUrl
    ? `<script type="module" src="${escapeHtml(options.clientScriptUrl)}"></script>`
    : "";

  return `</div>
<script>window.__FASTCART_RENDER__ = ${options.hydrationJson};</script>
${script}
</body>
</html>`;
}

/** Builds the complete HTML document around a rendered page tree. */
export function renderDocument(options: DocumentOptions): string {
  return `${renderDocumentPrefix(options)}${options.bodyHtml}${renderDocumentSuffix(options)}`;
}

/** Minimal document used for non-rendered statuses such as a missing page. */
export function renderErrorDocument(title: string, message?: string): string {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title>
</head>
<body>
<main>
<h1>${escapeHtml(title)}</h1>
${message ? `<p>${escapeHtml(message)}</p>` : ""}
</main>
</body>
</html>`;
}

/** Resolves the document title from SEO metadata or the page title. */
export function resolveTitle(page: { title?: string; seo?: unknown }): string {
  const seo = isRecord(page.seo) ? page.seo : {};
  const seoTitle = firstString(seo, ["title", "meta_title", "metaTitle"]);
  return seoTitle ?? page.title?.trim() ?? "";
}

function renderSeoHead(value: unknown): string {
  const seo = isRecord(value) ? value : {};
  const tags: string[] = [];
  const description = firstString(seo, ["description", "meta_description"]);
  if (description) {
    tags.push(metaName("description", description));
  }
  const keywords = seo.keywords;
  if (typeof keywords === "string" && keywords.trim().length > 0) {
    tags.push(metaName("keywords", keywords.trim()));
  } else if (Array.isArray(keywords)) {
    const joined = keywords.filter((item): item is string => typeof item === "string");
    if (joined.length > 0) {
      tags.push(metaName("keywords", joined.join(", ")));
    }
  }

  const noindex =
    seo.noindex === true || (typeof seo.robots === "string" && seo.robots.includes("noindex"));
  const robots = firstString(seo, ["robots"]) ?? (noindex ? "noindex, nofollow" : undefined);
  if (robots) {
    tags.push(metaName("robots", robots));
  }

  const canonical = firstString(seo, ["canonical", "canonical_url", "canonicalUrl"]);
  if (canonical) {
    tags.push(`<link rel="canonical" href="${escapeHtml(canonical)}">`);
  }

  const ogTitle = firstString(seo, ["og_title", "ogTitle"]);
  const ogDescription = firstString(seo, ["og_description", "ogDescription"]);
  const ogImage = firstString(seo, ["og_image", "ogImage", "image"]);
  const ogUrl = firstString(seo, ["og_url", "ogUrl", "url"]);
  const ogType = firstString(seo, ["og_type", "ogType"]);
  if (ogTitle) tags.push(property("og:title", ogTitle));
  if (ogDescription) tags.push(property("og:description", ogDescription));
  if (ogImage) tags.push(property("og:image", ogImage));
  if (ogUrl) tags.push(property("og:url", ogUrl));
  if (ogType) tags.push(property("og:type", ogType));

  const twitterCard = firstString(seo, ["twitter_card", "twitterCard"]);
  const twitterTitle = firstString(seo, ["twitter_title", "twitterTitle"]);
  const twitterDescription = firstString(seo, ["twitter_description", "twitterDescription"]);
  const twitterImage = firstString(seo, ["twitter_image", "twitterImage"]);
  if (twitterCard) tags.push(metaName("twitter:card", twitterCard));
  if (twitterTitle) tags.push(metaName("twitter:title", twitterTitle));
  if (twitterDescription) {
    tags.push(metaName("twitter:description", twitterDescription));
  }
  if (twitterImage) tags.push(metaName("twitter:image", twitterImage));

  return tags.join("\n");
}

function metaName(name: string, content: string): string {
  return `<meta name="${escapeHtml(name)}" content="${escapeHtml(content)}">`;
}

function property(name: string, content: string): string {
  return `<meta property="${escapeHtml(name)}" content="${escapeHtml(content)}">`;
}

function firstString(source: Record<string, unknown>, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = source[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim();
    }
  }
  return undefined;
}

/** Builds CSS custom properties from theme settings plus base theme styles. */
export function renderThemeStyle(settings: Record<string, unknown>): string {
  const variables: string[] = [];
  for (const [key, value] of Object.entries(settings)) {
    if (!/^[A-Za-z][A-Za-z0-9_-]*$/.test(key)) {
      continue;
    }
    if (typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean") {
      continue;
    }
    const safe = String(value)
      .replace(/[;{}<>]/g, "")
      .replace(/[\r\n]/g, " ")
      .trim()
      .slice(0, 200);
    if (safe.length === 0) {
      continue;
    }
    variables.push(`--fc-${kebabCase(key)}: ${safe};`);
  }

  return `:root {
${variables.map((line) => `  ${line}`).join("\n")}
  color-scheme: light;
}
*, *::before, *::after { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--fc-background-color, #ffffff);
  color: var(--fc-text-color, #111111);
  font-family: var(--fc-font-family, system-ui, -apple-system, "Segoe UI", sans-serif);
  line-height: 1.6;
}
.fc-page {
  width: 100%;
  max-width: var(--fc-container-width, 1200px);
  margin: 0 auto;
  padding: 0 1rem;
}
.fc-block + .fc-block { margin-top: 1rem; }
.fc-zone[data-zone="header"],
.fc-zone[data-zone="announcement"] { border-bottom: 1px solid #e5e5e5; padding: 0.75rem 0; }
.fc-zone[data-zone="footer"] { border-top: 1px solid #e5e5e5; margin-top: 3rem; padding: 1.5rem 0; }
.fc-page a { color: var(--fc-primary-color, #111111); }
.fc-page img { max-width: 100%; height: auto; }
.fc-block-unknown { display: none; }`;
}

function kebabCase(value: string): string {
  return value
    .replace(/_/g, "-")
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .toLowerCase();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
