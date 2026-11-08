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
.fc-block-unknown { display: none; }

.fc-button {
  appearance: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.4rem;
  min-height: 2.5rem;
  padding: 0.55rem 0.9rem;
  border: 1px solid #d4d4d4;
  border-radius: 0.5rem;
  background: #ffffff;
  color: inherit;
  font: inherit;
  font-weight: 600;
  line-height: 1.2;
  text-align: center;
  text-decoration: none;
  cursor: pointer;
}
.fc-button--primary {
  border-color: var(--fc-primary-color, #111111);
  background: var(--fc-primary-color, #111111);
  color: #ffffff;
}
.fc-button--secondary { background: transparent; }
.fc-button--ghost {
  min-height: 0;
  padding: 0.3rem 0.4rem;
  border-color: transparent;
  background: transparent;
}
.fc-button--small {
  min-height: 2rem;
  padding: 0.35rem 0.65rem;
  font-size: 0.875rem;
}
.fc-button:disabled { opacity: 0.55; cursor: not-allowed; }
.fc-button:focus-visible,
.fc-cart-field input:focus-visible,
.fc-cart-field select:focus-visible,
.fc-quantity-field input:focus-visible {
  outline: 2px solid var(--fc-primary-color, #111111);
  outline-offset: 2px;
}
.fc-status { margin: 0; font-size: 0.875rem; }
.fc-status--success { color: #15803d; }
.fc-status--error { color: #b91c1c; }

.fc-product-detail {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 2rem;
  align-items: start;
}
.fc-product-gallery {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
  gap: 0.75rem;
  margin-top: 1.5rem;
}
.fc-product-purchase {
  display: grid;
  gap: 0.85rem;
  margin-top: 1.5rem;
}
.fc-variant-picker {
  display: grid;
  gap: 0.5rem;
  margin: 0;
  padding: 0.85rem;
  border: 1px solid #e5e5e5;
  border-radius: 0.5rem;
}
.fc-variant-picker legend { padding: 0 0.25rem; font-weight: 600; }
.fc-variant-picker label { display: flex; align-items: center; gap: 0.5rem; }
.fc-product-buy-row {
  display: flex;
  flex-wrap: wrap;
  align-items: end;
  gap: 0.75rem;
}
.fc-quantity-field { display: grid; gap: 0.25rem; font-size: 0.875rem; }
.fc-quantity-field input {
  width: 5rem;
  min-height: 2.5rem;
  padding: 0.45rem 0.55rem;
  border: 1px solid #d4d4d4;
  border-radius: 0.375rem;
  background: #ffffff;
  color: inherit;
  font: inherit;
}
.fc-product-card-action { display: grid; gap: 0.35rem; margin-top: 0.5rem; }

.fc-cart-launcher {
  position: fixed;
  right: 1rem;
  bottom: 1rem;
  z-index: 900;
  box-shadow: 0 10px 25px rgba(0, 0, 0, 0.18);
}
.fc-cart-launcher-count {
  display: inline-grid;
  place-items: center;
  min-width: 1.5rem;
  height: 1.5rem;
  padding: 0 0.35rem;
  border-radius: 999px;
  background: var(--fc-background-color, #ffffff);
  color: var(--fc-primary-color, #111111);
  font-size: 0.8rem;
  font-variant-numeric: tabular-nums;
}
.fc-cart-layer {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  justify-content: flex-end;
}
.fc-cart-backdrop {
  position: absolute;
  inset: 0;
  background: rgba(17, 17, 17, 0.45);
}
.fc-cart-drawer {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  width: min(28rem, 100%);
  height: 100%;
  overflow: hidden;
  background: var(--fc-background-color, #ffffff);
  color: var(--fc-text-color, #111111);
  box-shadow: -12px 0 35px rgba(0, 0, 0, 0.22);
  outline: none;
}
.fc-cart-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  padding: 1rem 1.25rem;
  border-bottom: 1px solid #e5e5e5;
}
.fc-cart-header h2 { margin: 0; font-size: 1.25rem; line-height: 1.3; }
.fc-cart-close { min-width: 2.5rem; font-size: 1.5rem; line-height: 1; }
.fc-cart-error {
  margin: 0;
  padding: 0.75rem 1.25rem;
  border-bottom: 1px solid #fecaca;
  background: #fef2f2;
  color: #b91c1c;
  font-size: 0.9rem;
}
.fc-cart-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 1rem 1.25rem;
}
.fc-cart-empty { display: grid; align-content: start; gap: 0.75rem; }
.fc-cart-hint { margin: 0; color: #525252; font-size: 0.9rem; }
.fc-cart-loading { margin: 0; color: #525252; }
.fc-cart-lines {
  display: grid;
  gap: 1rem;
  margin: 0;
  padding: 0;
  list-style: none;
}
.fc-cart-line {
  display: grid;
  grid-template-columns: 4rem minmax(0, 1fr) auto;
  gap: 0.75rem;
  padding-bottom: 1rem;
  border-bottom: 1px solid #eeeeee;
}
.fc-cart-line img,
.fc-cart-line-thumb {
  width: 4rem;
  height: 4rem;
  border-radius: 0.375rem;
  background: #f5f5f5;
  object-fit: cover;
}
.fc-cart-line-title { margin: 0; font-weight: 600; }
.fc-cart-line-meta { margin: 0.15rem 0; color: #525252; font-size: 0.875rem; }
.fc-cart-line-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.5rem;
  margin-top: 0.5rem;
}
.fc-cart-qty {
  display: inline-flex;
  align-items: center;
  overflow: hidden;
  border: 1px solid #d4d4d4;
  border-radius: 0.375rem;
}
.fc-cart-qty button {
  width: 2rem;
  height: 2rem;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}
.fc-cart-qty button:disabled { opacity: 0.4; cursor: not-allowed; }
.fc-cart-qty span {
  min-width: 2rem;
  text-align: center;
  font-variant-numeric: tabular-nums;
}
.fc-cart-remove { color: #b91c1c; }
.fc-cart-line-total {
  margin: 0;
  font-weight: 600;
  white-space: nowrap;
}
.fc-cart-footer {
  display: grid;
  gap: 0.75rem;
  padding: 1rem 1.25rem;
  border-top: 1px solid #e5e5e5;
  background: #fafafa;
}
.fc-cart-totals {
  display: grid;
  gap: 0.35rem;
  margin: 0;
}
.fc-cart-totals > div {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
}
.fc-cart-totals dt,
.fc-cart-totals dd { margin: 0; }
.fc-cart-totals dd { text-align: right; font-variant-numeric: tabular-nums; }
.fc-cart-grand-total {
  padding-top: 0.5rem;
  border-top: 1px solid #e5e5e5;
  font-size: 1.05rem;
  font-weight: 700;
}
.fc-cart-button-row { display: flex; flex-wrap: wrap; gap: 0.5rem; }
.fc-cart-button-row .fc-button { flex: 1 1 auto; }
.fc-cart-form { display: flex; flex: 1; flex-direction: column; min-height: 0; }
.fc-cart-form .fc-cart-scroll { display: grid; align-content: start; gap: 1rem; }
.fc-cart-field { display: grid; gap: 0.25rem; }
.fc-cart-field label { font-size: 0.875rem; font-weight: 600; }
.fc-cart-field input,
.fc-cart-field select {
  width: 100%;
  min-height: 2.5rem;
  padding: 0.5rem 0.6rem;
  border: 1px solid #d4d4d4;
  border-radius: 0.375rem;
  background: #ffffff;
  color: inherit;
  font: inherit;
}
.fc-cart-field-row {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.75rem;
}
.fc-cart-field-error { margin: 0; color: #b91c1c; font-size: 0.8rem; }
.fc-cart-section { margin: 0 0 1.25rem; }
.fc-cart-section h3 { margin: 0 0 0.35rem; font-size: 1rem; }
.fc-cart-section-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.75rem;
}
.fc-cart-address { margin: 0; color: #525252; }
.fc-cart-shipping-options {
  display: grid;
  gap: 0.5rem;
  margin: 0;
  padding: 0.85rem;
  border: 1px solid #e5e5e5;
  border-radius: 0.5rem;
}
.fc-cart-shipping-options legend { padding: 0 0.25rem; font-weight: 600; }
.fc-cart-shipping-option {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0;
}
.fc-cart-shipping-option input { flex: none; }
.fc-cart-shipping-option-name { flex: 1; }
.fc-cart-confirmation { display: grid; gap: 1rem; }
.fc-cart-eyebrow {
  margin: 0;
  color: #525252;
  font-size: 0.75rem;
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

@media (max-width: 720px) {
  .fc-product-detail { grid-template-columns: minmax(0, 1fr); gap: 1.25rem; }
}
@media (max-width: 480px) {
  .fc-cart-drawer { width: 100%; }
  .fc-cart-field-row { grid-template-columns: minmax(0, 1fr); }
  .fc-cart-line { grid-template-columns: 3.25rem minmax(0, 1fr); }
  .fc-cart-line img,
  .fc-cart-line-thumb { width: 3.25rem; height: 3.25rem; }
  .fc-cart-line-total { grid-column: 2; }
}`;
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
