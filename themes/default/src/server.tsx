import type { ReactElement } from "react";
import { renderToReadableStream } from "react-dom/server.browser";

import type { RenderPayload, RenderResult } from "@fastcart/theme-sdk";

import { PageRenderer } from "./components/page-renderer";
import {
  renderDocumentPrefix,
  renderDocumentSuffix,
  renderErrorDocument,
  resolveTitle,
  type DocumentShellOptions,
} from "./document";
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
const NOT_FOUND_HEADERS: [string, string][] = [[CONTENT_TYPE, HTML_UTF8]];
const PAGE_HEADERS: [string, string][] = [
  [CONTENT_TYPE, HTML_UTF8],
  ["x-content-type-options", "nosniff"],
];

/**
 * Renders one storefront request for the Node sidecar.
 *
 * The host sends a validated page body, resolved data, theme settings and the
 * public asset base. The document head and shell are streamed as soon as
 * React is ready; the page tree follows through the same body stream. When
 * the shell cannot render, the function falls back to a buffered error
 * document so the host can still return a real status code.
 */
export async function render(payload: RenderPayload): Promise<RenderResult> {
  const page = payload.page;
  if (!page) {
    return {
      status: 404,
      headers: NOT_FOUND_HEADERS,
      body: renderErrorDocument("Page not found"),
    };
  }

  const body = normalizePageBody(page.body);
  const settings = normalizeSettings(payload.theme?.settings);
  const data = isRecord(payload.data) ? payload.data : null;
  const mode = normalizeMode(payload.mode);
  const apiBaseUrl = apiBaseUrlFromSettings(settings);
  const assetsBaseUrl = payload.theme?.assets_base_url ?? undefined;

  const hydration: HydrationState = {
    body,
    data,
    settings,
    mode,
    ...(apiBaseUrl ? { apiBaseUrl } : {}),
    ...(assetsBaseUrl ? { assetsBaseUrl } : {}),
  };

  const shell: DocumentShellOptions = {
    lang: payload.store?.locale ?? "en",
    title: resolveTitle(page) || page.handle || "Fastcart",
    seo: page.seo,
    settings,
    hydrationJson: serializeHydrationState(hydration),
    clientScriptUrl: clientScriptUrl(payload.theme),
  };

  return renderStreamingPage({
    shell,
    headers: [
      ...PAGE_HEADERS,
      ["cache-control", mode === "production" ? "public, max-age=0, must-revalidate" : "no-store"],
    ],
    page: (
      <PageRenderer
        body={body}
        data={data}
        settings={settings}
        apiBaseUrl={apiBaseUrl}
        assetsBaseUrl={assetsBaseUrl}
        mode={mode}
      />
    ),
  });
}

interface StreamingPageInput {
  shell: DocumentShellOptions;
  headers: [string, string][];
  page: ReactElement;
}

/**
 * Streams a complete document around a React tree.
 *
 * `renderToReadableStream` resolves once the shell is ready, so a shell error
 * can still produce a buffered 500 instead of a half-written 200. The
 * document prefix is yielded before the React chunks and the closing markup
 * plus hydration state after them; the async generator propagates backpressure
 * from the sidecar to React.
 */
async function renderStreamingPage(input: StreamingPageInput): Promise<RenderResult> {
  let reactStream: ReadableStream<Uint8Array>;
  try {
    reactStream = await renderToReadableStream(input.page, {
      onError(error: unknown) {
        console.error("theme render error", error);
      },
    });
  } catch (error) {
    console.error("theme shell render failed", error);
    return {
      status: 500,
      headers: PAGE_HEADERS,
      body: renderErrorDocument("Something went wrong"),
    };
  }

  return {
    status: 200,
    headers: input.headers,
    body: documentStream(reactStream, input.shell),
  };
}

async function* documentStream(
  reactStream: ReadableStream<Uint8Array>,
  shell: DocumentShellOptions,
): AsyncIterable<Uint8Array> {
  const encoder = new TextEncoder();
  yield encoder.encode(renderDocumentPrefix(shell));
  const reader = reactStream.getReader();
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      yield value;
    }
  } finally {
    reader.releaseLock();
  }
  yield encoder.encode(renderDocumentSuffix(shell));
}

export default { render };
