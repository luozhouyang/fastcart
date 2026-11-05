import { renderToString } from "react-dom/server";

import type { RenderPayload, RenderResult } from "@fastcart/theme-sdk";

import { PageRenderer } from "./components/page-renderer";
import { renderDocument, renderErrorDocument, resolveTitle } from "./document";
import type { HydrationState } from "./hydration";
import { serializeHydrationState } from "./hydration";
import {
  apiBaseUrlFromSettings,
  clientScriptUrl,
  isRecord,
  normalizeMode,
  normalizePageBody,
  normalizeSettings,
} from "./runtime";

const CONTENT_TYPE = "content-type";
const HTML_UTF8 = "text/html; charset=utf-8";

/**
 * Renders one storefront request for the Node sidecar.
 *
 * The host sends a validated page body, resolved data, theme settings and the
 * public asset base. This function returns a complete HTML document; the
 * sidecar base64-encodes the body for the Rust `NodeRuntime`.
 */
export async function render(payload: RenderPayload): Promise<RenderResult> {
  const page = payload.page;
  if (!page) {
    return {
      status: 404,
      headers: [[CONTENT_TYPE, HTML_UTF8]],
      body: renderErrorDocument("Page not found"),
    };
  }

  const body = normalizePageBody(page.body);
  const settings = normalizeSettings(payload.theme?.settings);
  const data = isRecord(payload.data) ? payload.data : null;
  const mode = normalizeMode(payload.mode);
  const apiBaseUrl = apiBaseUrlFromSettings(settings);
  const assetsBaseUrl = payload.theme?.assets_base_url ?? undefined;

  const bodyHtml = renderToString(
    <PageRenderer
      body={body}
      data={data}
      settings={settings}
      apiBaseUrl={apiBaseUrl}
      assetsBaseUrl={assetsBaseUrl}
      mode={mode}
    />,
  );

  const hydration: HydrationState = {
    body,
    data,
    settings,
    mode,
    ...(apiBaseUrl ? { apiBaseUrl } : {}),
    ...(assetsBaseUrl ? { assetsBaseUrl } : {}),
  };

  const html = renderDocument({
    lang: payload.store?.locale ?? "en",
    title: resolveTitle(page) || page.handle || "Fastcart",
    seo: page.seo,
    settings,
    bodyHtml,
    hydrationJson: serializeHydrationState(hydration),
    clientScriptUrl: clientScriptUrl(payload.theme),
  });

  return {
    status: 200,
    headers: [
      [CONTENT_TYPE, HTML_UTF8],
      ["cache-control", mode === "production" ? "public, max-age=0, must-revalidate" : "no-store"],
      ["x-content-type-options", "nosniff"],
    ],
    body: html,
  };
}

export default { render };
