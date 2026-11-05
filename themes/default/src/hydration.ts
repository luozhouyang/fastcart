import type { PageBody, RenderMode } from "@fastcart/theme-sdk";

/**
 * State embedded in the server-rendered document and consumed by
 * `client.tsx` during hydration. Keep this JSON-serializable: it is inlined
 * into a `<script>` in the document.
 */
export interface HydrationState {
  body: PageBody;
  data: Record<string, unknown> | null;
  settings: Record<string, unknown>;
  mode: RenderMode;
  apiBaseUrl?: string;
  assetsBaseUrl?: string;
}

declare global {
  interface Window {
    __FASTCART_RENDER__?: HydrationState;
  }
}

/** Serializes state so it cannot terminate the surrounding script element. */
export function serializeHydrationState(state: HydrationState): string {
  const lineSeparator = String.fromCharCode(0x2028);
  const paragraphSeparator = String.fromCharCode(0x2029);
  return JSON.stringify(state)
    .replace(/</g, "\\u003c")
    .split(lineSeparator)
    .join("\\u2028")
    .split(paragraphSeparator)
    .join("\\u2029");
}

/** Reads the state embedded by the server renderer. */
export function readHydrationState(): HydrationState | null {
  if (typeof window === "undefined") {
    return null;
  }
  return window.__FASTCART_RENDER__ ?? null;
}
